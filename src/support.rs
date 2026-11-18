//! Game-side glue for Simple Backups: config, world tracking, the worker, the commands.
//!
//! The rules — schedule arithmetic, names, retention, skip rules — are [`crate::runtime`]'s and
//! are byte-identical to the retired bundle. This module is only the transport:
//!
//! 1. `server_about_to_start` reads `config/simplebackups.toml` through the host's checked,
//!    mod-owned config service, writing the defaults when it is absent.
//! 2. The first running tick of a world takes `host.world_dir()` (there is no open phase at
//!    this pin) and resets the schedule.
//! 3. At the end of each running tick the worker is polled; a due or queued backup saves the
//!    open levels (`LevelRef::request_save`, whose save runs at the next tick's start) and starts
//!    the copy when the save cycle completes, or starts it at once when `save_before_backup` is
//!    off.
//! 4. The worker walks the world folder into `<world>_<UTC>.zip.part` with [`crate::zip`], the
//!    tick publishes it and prunes, and a copy a save overlapped is thrown away and redone.

use std::fs::{self, File};
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use mod_api::{
    Command, ConfigFileError, ConfigPath, Flow, Host, RunModCommandEvent, ServerHost, Text,
};

use crate::runtime::{
    Config, backup_file_name, copy_is_stale, due_by_millis, due_by_ticks, entry_name, format_size,
    parse, prune, should_skip, to_toml,
};
use crate::zip::ZipWriter;

/// The port's config file in the game's `config/` folder: `<mod id>.toml`, which the service
/// hands to the mod that owns it.
const CONFIG_FILE: &str = "simplebackups.toml";

/// How many times a copy a save overlapped is redone before it is published anyway (each retry
/// costs another save and another copy, so the cap is small).
const MAX_RETRIES: u32 = 2;

/// How many ticks a requested save may take before the copy starts without it. A save request
/// that never reports back must not stall the schedule for ever.
const SAVE_WAIT_TICKS: u64 = 200;

/// A finished copy of the world as a `.zip.part`, not yet published: the server tick that
/// receives it checks whether a save overlapped it before it renames the part (and prunes).
struct Copy {
    part: PathBuf,
    final_path: PathBuf,
    bytes: u64,
}

struct Report {
    name: String,
    bytes: u64,
    deleted: Vec<String>,
    /// Published even though a save overlapped it (retries exhausted or off).
    stale: bool,
}

#[derive(Default)]
struct Core {
    cfg: Config,
    /// The open world's folder, taken at the first running tick of the world.
    world: Option<PathBuf>,
    ticks: u64,
    last_tick: u64,
    last_ms: u64,
    in_flight: bool,
    /// The in-flight request was quiet (no broadcast).
    in_flight_quiet: bool,
    /// `world_saved` notifications seen, the port's own saves included.
    save_seq: u64,
    /// `save_seq` when the in-flight copy started.
    save_seq_at_start: u64,
    /// Copies redone because a save overlapped them (per backup).
    retries: u32,
    /// The queued request redoes a copy that overlapped a save.
    retrying: bool,
    /// The last published copy had overlapped a save and could not be redone.
    stale: bool,
    /// A manual request: `Some(quiet)`.
    pending: Option<bool>,
    /// A backup is waiting for its save cycle to complete; `Some(quiet)`.
    waiting_save: Option<bool>,
    /// Ticks spent waiting for that cycle.
    save_wait_ticks: u64,
    rx: Option<Receiver<Result<Copy, String>>>,
    closing: bool,
    last_completed: Option<String>,
    last_error: Option<String>,
    /// Config lines the parser refused, shown by `/simplebackups status`.
    warnings: Vec<String>,
}

/// The session state. `Hooks: Send + Sync`, so it is a `Mutex`, and the macro boxes one
/// `Default` instance for the session, so the state survives across calls and dies with it.
static CORE: Mutex<Option<Core>> = Mutex::new(None);

fn core() -> std::sync::MutexGuard<'static, Option<Core>> {
    CORE.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// Before the server opens anything: read `config/simplebackups.toml`, writing the defaults when
/// it is absent. A file the service refuses to read gives the defaults, which is what a missing
/// file gives too; a refused default write only means the next start reads the defaults again.
pub fn load_config(host: &dyn Host) {
    let Ok(path) = ConfigPath::new(CONFIG_FILE) else {
        return;
    };
    let files = host.config_files();
    let (cfg, warnings) = match files.read(&path) {
        Ok(bytes) => parse(&String::from_utf8_lossy(&bytes)),
        Err(ConfigFileError::NotFound) => {
            let cfg = Config::default();
            let _ = files.write(&path, to_toml(&cfg).as_bytes());
            (cfg, Vec::new())
        }
        Err(_) => {
            let cfg = Config::default();
            let text = to_toml(&cfg);
            let _ = files.write(&path, text.as_bytes());
            parse(&text)
        }
    };
    let mut guard = core();
    let core = guard.get_or_insert_with(Core::default);
    *core = Core {
        cfg,
        warnings,
        ..Core::default()
    };
}

/// The world's phases, read-only. There is no "opened" phase at this pin, so this only forgets
/// the world: the next one is taken at its first running tick by [`ensure_world`].
pub fn on_world_lifecycle(phase: mod_api::WorldPhase) {
    let mut guard = core();
    let core = guard.get_or_insert_with(Core::default);
    match phase {
        mod_api::WorldPhase::WorldCloseFinal | mod_api::WorldPhase::WorldCloseFailed => {
            core.world = None;
            core.closing = true;
            abandon_save_wait(core);
        }
        mod_api::WorldPhase::WorldOpenFailed => {
            core.closing = true;
            abandon_save_wait(core);
        }
        // The list is not exhaustive: a later phase that is neither a close nor a failed open
        // changes nothing here.
        _ => {}
    }
}

/// The world went away while a save was still awaited: nothing will report it back, so drop the
/// wait instead of letting the next world's ticks start a copy for a world that closed. A worker
/// that is already copying stays in flight until its result arrives.
fn abandon_save_wait(core: &mut Core) {
    if core.waiting_save.take().is_some() && core.rx.is_none() {
        core.in_flight = false;
    }
    core.save_wait_ticks = 0;
}

/// The server is stopping: no new backups.
pub fn on_server_stopping() {
    let mut guard = core();
    let core = guard.get_or_insert_with(Core::default);
    core.closing = true;
    abandon_save_wait(core);
}

/// Take the open world's folder once per world, and start the schedule from zero.
fn ensure_world(host: &dyn ServerHost) {
    {
        let guard = core();
        if guard.as_ref().is_some_and(|core| core.world.is_some()) {
            return;
        }
    }
    let Some(dir) = host.world_dir() else {
        return;
    };
    let mut guard = core();
    let core = guard.get_or_insert_with(Core::default);
    if core.world.as_deref() == Some(dir.as_path()) {
        return;
    }
    core.world = Some(dir);
    core.ticks = 0;
    core.last_tick = 0;
    core.last_ms = now_ms();
    core.closing = false;
    core.last_error = None;
    core.save_seq = 0;
    core.save_seq_at_start = 0;
    core.retries = 0;
    core.retrying = false;
    core.stale = false;
}

/// The server completed a save cycle (`world_saved`): the port's own requested save and every
/// autosave reach it. The sequence lets a finished copy tell whether a save landed while it was
/// in flight; a backup that was waiting for its save starts now.
pub fn on_world_saved(host: &mut dyn ServerHost, _wait: bool) {
    let waiting = {
        let mut guard = core();
        let Some(core) = guard.as_mut() else {
            return;
        };
        core.save_seq = core.save_seq.wrapping_add(1);
        core.waiting_save.take()
    };
    if let Some(quiet) = waiting {
        let mut guard = core();
        let core = guard.get_or_insert_with(Core::default);
        core.save_wait_ticks = 0;
        drop(guard);
        begin_copy(host, quiet);
    }
}

/// End of a running server tick: poll the worker and start the next backup.
pub fn on_server_tick(host: &mut dyn ServerHost, world_running: bool) {
    ensure_world(host);
    if !world_running {
        return;
    }

    // A save the port asked for that never reports back must not stall the schedule.
    let expired = {
        let mut guard = core();
        let Some(core) = guard.as_mut() else {
            return;
        };
        if core.waiting_save.is_some() {
            core.save_wait_ticks += 1;
            if core.save_wait_ticks > SAVE_WAIT_TICKS {
                core.waiting_save.take()
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some(quiet) = expired {
        let mut guard = core();
        if let Some(core) = guard.as_mut() {
            core.save_wait_ticks = 0;
            core.last_error = Some("the save did not report back; copied anyway".to_owned());
        }
        drop(guard);
        begin_copy(host, quiet);
    }

    // Poll the worker result.
    let finished = {
        let mut guard = core();
        let Some(core) = guard.as_mut() else {
            return;
        };
        if let Some(rx) = core.rx.take() {
            match rx.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => {
                    core.rx = Some(rx);
                    None
                }
                Err(TryRecvError::Disconnected) => {
                    Some(Err("the backup worker stopped".to_owned()))
                }
            }
        } else {
            None
        }
    };
    if let Some(result) = finished {
        // A copy that overlapped a save is thrown away and redone; only a published copy (or a
        // failure) is reported.
        let outcome = {
            let mut guard = core();
            let core = guard.get_or_insert_with(Core::default);
            core.in_flight = false;
            match result {
                Ok(copy) => {
                    let stale = copy_is_stale(core.save_seq_at_start, core.save_seq);
                    if stale && core.cfg.retry_on_save && core.retries < MAX_RETRIES {
                        let _ = fs::remove_file(&copy.part);
                        core.retries += 1;
                        core.retrying = true;
                        core.pending = Some(core.in_flight_quiet);
                        None
                    } else {
                        Some(publish(core, &copy, stale))
                    }
                }
                Err(error) => Some(Err(error)),
            }
        };
        if let Some(outcome) = outcome {
            let (message, quiet) = {
                let mut guard = core();
                let core = guard.get_or_insert_with(Core::default);
                match outcome {
                    Ok(report) => {
                        let mut message =
                            format!("Backup {} ({})", report.name, format_size(report.bytes));
                        if !report.deleted.is_empty() {
                            message.push_str(&format!("; pruned {}", report.deleted.len()));
                        }
                        if report.stale {
                            message.push_str("; a save overlapped this copy");
                        }
                        core.last_completed = Some(report.name);
                        core.last_error = None;
                        (message, !core.cfg.broadcast)
                    }
                    Err(error) => {
                        core.last_error = Some(error.clone());
                        (format!("Backup failed: {error}"), !core.cfg.broadcast)
                    }
                }
            };
            if !quiet {
                let _ = host.messages().broadcast(Text::literal(message));
            }
        }
    }

    let manual = {
        let mut guard = core();
        let Some(core) = guard.as_mut() else {
            return;
        };
        core.ticks += 1;
        if core.closing {
            core.pending = None;
            return;
        }
        let manual = core.pending.take();
        if core.in_flight || core.waiting_save.is_some() {
            if let Some(quiet) = manual {
                core.pending = Some(quiet);
            }
            return;
        }
        let due = core.cfg.enabled
            && if core.cfg.interval_ticks > 0 {
                due_by_ticks(core.last_tick, core.ticks, core.cfg.interval_ticks)
            } else {
                due_by_millis(core.last_ms, now_ms(), core.cfg.interval_minutes)
            };
        if manual.is_none() && !due {
            return;
        }
        manual
    };
    let Some(quiet) = manual else {
        // Scheduled: not quiet.
        start_backup(host, false);
        return;
    };
    start_backup(host, quiet);
}

/// A backup is due or queued: save the open levels (or not), then start the copy.
fn start_backup(host: &mut dyn ServerHost, quiet: bool) {
    let (cfg, world) = {
        let mut guard = core();
        let Some(core) = guard.as_mut() else {
            return;
        };
        // A retry keeps the retry count; a fresh request starts at zero.
        if core.retrying {
            core.retrying = false;
        } else {
            core.retries = 0;
            core.stale = false;
        }
        (core.cfg.clone(), core.world.clone())
    };
    let Some(_world) = world else {
        let mut guard = core();
        if let Some(core) = guard.as_mut() {
            core.last_error = Some("no world open".to_owned());
        }
        return;
    };
    {
        let mut guard = core();
        if let Some(core) = guard.as_mut() {
            core.last_tick = core.ticks;
            core.last_ms = now_ms();
            core.in_flight = true;
            core.in_flight_quiet = quiet;
            core.pending = None;
        }
    }
    if !cfg.save_before_backup {
        begin_copy(host, quiet);
        return;
    }
    // Every open level is asked for, so the copy sees flushed chunks in all of them. The save
    // runs at the start of the next tick, and `world_saved` completes the request.
    let levels = host.levels().list_levels();
    let mut requested = false;
    for level in levels {
        requested |= level.request_save(host);
    }
    if !requested {
        // A stale handle or no level: nothing will report back, so copy now (as with
        // `save_before_backup = false`).
        begin_copy(host, quiet);
        return;
    }
    let mut guard = core();
    if let Some(core) = guard.as_mut() {
        core.waiting_save = Some(quiet);
        core.save_wait_ticks = 0;
    }
}

/// Start the worker for the in-flight request: name the file, spawn the copy, tell the players.
fn begin_copy(host: &mut dyn ServerHost, quiet: bool) {
    let (cfg, world, name, final_path) = {
        let mut guard = core();
        let Some(core) = guard.as_mut() else {
            return;
        };
        let Some(world) = core.world.clone() else {
            core.in_flight = false;
            core.last_error = Some("no world open".to_owned());
            return;
        };
        let backup_dir = world.join(core.cfg.backups_dir());
        if let Err(error) = fs::create_dir_all(&backup_dir) {
            core.in_flight = false;
            core.last_error = Some(error.to_string());
            return;
        }
        let world_name = world_name(&world);
        let taken: Vec<String> = fs::read_dir(&backup_dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        let name = backup_file_name(&world_name, now_secs(), &taken);
        let final_path = backup_dir.join(&name);
        core.in_flight = true;
        core.in_flight_quiet = quiet;
        core.save_seq_at_start = core.save_seq;
        (core.cfg.clone(), world, name, final_path)
    };

    let (tx, rx) = mpsc::channel();
    let worker_cfg = cfg.clone();
    let worker_final = final_path.clone();
    thread::spawn(move || {
        let result = run_backup(&world, &worker_final, &worker_cfg);
        let _ = tx.send(result);
    });
    {
        let mut guard = core();
        if let Some(core) = guard.as_mut() {
            core.rx = Some(rx);
        }
    }
    if !quiet && cfg.broadcast {
        let _ = host
            .messages()
            .broadcast(Text::literal(format!("Backup {name} started")));
    }
}

/// Walk the world into `<final>.part` and return it unpublished; the server tick renames it only
/// when no save overlapped the copy.
fn run_backup(world: &Path, final_path: &Path, cfg: &Config) -> Result<Copy, String> {
    let world_name = world_name(world);
    let part = final_path.with_extension("zip.part");
    let result = (|| -> Result<u64, String> {
        let file = File::create(&part).map_err(|error| error.to_string())?;
        let mut zip = ZipWriter::new(BufWriter::new(file));
        walk(
            &mut zip,
            world,
            "",
            &world_name,
            cfg,
            Some(cfg.backups_dir()),
        )?;
        let writer = zip.finish().map_err(|error| error.to_string())?;
        let file = writer
            .into_inner()
            .map_err(|error| error.error().to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        Ok(fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0))
    })();
    let bytes = match result {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = fs::remove_file(&part);
            return Err(error);
        }
    };
    Ok(Copy {
        part,
        final_path: final_path.to_owned(),
        bytes,
    })
}

/// Publish a finished copy: rename the part, prune old backups, report.
fn publish(core: &mut Core, copy: &Copy, stale: bool) -> Result<Report, String> {
    let Some(world) = core.world.clone() else {
        let _ = fs::remove_file(&copy.part);
        return Err("no world open".to_owned());
    };
    let world_name = world_name(&world);
    let backups = world.join(core.cfg.backups_dir());
    fs::rename(&copy.part, &copy.final_path).map_err(|error| {
        let _ = fs::remove_file(&copy.part);
        error.to_string()
    })?;
    let names: Vec<String> = fs::read_dir(&backups)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let deleted = prune(&names, &world_name, core.cfg.keep);
    for name in &deleted {
        let _ = fs::remove_file(backups.join(name));
    }
    if stale {
        core.stale = true;
    }
    Ok(Report {
        name: copy
            .final_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        bytes: copy.bytes,
        deleted,
        stale,
    })
}

/// The world folder's own name, which prefix every entry and every backup file name.
fn world_name(world: &Path) -> String {
    world
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "world".to_owned())
}

/// One directory level of the walk: entries in name order, directories first-class, symlinks,
/// the backups folder and anything the config ignores left out.
fn walk<W: io::Write + io::Seek>(
    zip: &mut ZipWriter<W>,
    dir: &Path,
    prefix: &str,
    world_name: &str,
    cfg: &Config,
    backups_prefix: Option<&str>,
) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|error| error.to_string())?
        .flatten()
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if should_skip(&relative, backups_prefix, cfg) {
            continue;
        }
        if metadata.is_dir() {
            walk(zip, &path, &relative, world_name, cfg, backups_prefix)?;
            continue;
        }
        if !metadata.is_file() {
            // A fifo or a socket would block the worker for ever; only files are archived.
            continue;
        }
        let Some(entry_name) = entry_name(world_name, &relative) else {
            continue;
        };
        match zip.add_file(&entry_name, &path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

/// Declares the `/simplebackups` commands (permission level 2).
pub fn register_commands(commands: &mut Vec<Command>) {
    commands.push(Command::op("simplebackups backup [quiet:boolean]", 2));
    commands.push(Command::op("simplebackups status", 2));
    commands.push(Command::op("simplebackups list", 2));
}

/// Runs the commands. `backup` only queues; the save and the worker start at the tick's end,
/// where a save request is safe.
pub fn run_command(host: &mut dyn ServerHost, event: &RunModCommandEvent<'_>) -> Flow {
    let literals: Vec<&str> = event.command.literals.iter().map(String::as_str).collect();
    match literals.as_slice() {
        ["simplebackups", "backup"] => {
            let quiet = matches!(
                event.command.argument("quiet"),
                Some(mod_api::CommandArgumentValue::Boolean(true))
            );
            {
                let mut guard = core();
                let core = guard.get_or_insert_with(Core::default);
                core.pending = Some(quiet);
            }
            let _ = host.commands().feedback(Text::literal("Backup queued."));
        }
        ["simplebackups", "status"] => {
            for line in status_lines() {
                let _ = host.commands().feedback(Text::literal(line));
            }
        }
        ["simplebackups", "list"] => {
            for line in list_lines() {
                let _ = host.commands().feedback(Text::literal(line));
            }
        }
        _ => return Flow::Continue,
    }
    Flow::Cancel
}

fn status_lines() -> Vec<String> {
    let guard = core();
    let Some(core) = guard.as_ref() else {
        return vec!["Simple Backups is not initialised.".to_owned()];
    };
    let mut lines = vec![format!(
        "Simple Backups: enabled={} interval={} keep={} compression={} save={}",
        core.cfg.enabled,
        if core.cfg.interval_ticks > 0 {
            format!("{} ticks", core.cfg.interval_ticks)
        } else {
            format!("{} minutes", core.cfg.interval_minutes)
        },
        core.cfg.keep,
        core.cfg.compression_level,
        core.cfg.save_before_backup
    )];
    if let Some(world) = &core.world {
        lines.push(format!(
            "world={} backups={}",
            world.display(),
            world.join(core.cfg.backups_dir()).display()
        ));
    }
    lines.push(format!(
        "in_flight={} last={} error={}",
        core.in_flight,
        core.last_completed.as_deref().unwrap_or("-"),
        core.last_error.as_deref().unwrap_or("-")
    ));
    lines.push(format!(
        "saves={} retries={} stale={}",
        core.save_seq, core.retries, core.stale
    ));
    // The 0.1.0 bundle logged config warnings; a native mod has no logger, so this is where they
    // reach a player. The four lines above are byte-identical to 0.1.0.
    for warning in &core.warnings {
        lines.push(format!("config: {warning}"));
    }
    lines
}

fn list_lines() -> Vec<String> {
    let guard = core();
    let Some(core) = guard.as_ref() else {
        return vec!["Simple Backups is not initialised.".to_owned()];
    };
    let Some(world) = &core.world else {
        return vec!["No world is open.".to_owned()];
    };
    let dir = world.join(core.cfg.backups_dir());
    let mut names: Vec<String> = fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".zip"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.reverse();
    if names.is_empty() {
        return vec![format!("No backups in {}.", dir.display())];
    }
    names
        .into_iter()
        .map(|name| {
            let size = fs::metadata(dir.join(&name))
                .map(|meta| format_size(meta.len()))
                .unwrap_or_else(|_| "?".to_owned());
            format!("{name} ({size})")
        })
        .collect()
}
