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

use std::fs::{self, OpenOptions};
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
    Settings, backup_file_name, cap_removals, copy_is_stale, due_by_millis, due_by_ticks,
    entry_name, format_size, format_size_setting, parse, prune, should_skip, to_toml,
};
use crate::zip::ZipWriter;

/// The port's config file in the game's `config/` folder: `<mod id>.toml`, which the service
/// hands to the mod that owns it.
const CONFIG_FILE: &str = "simplebackups.toml";

/// How many times a copy a save overlapped is redone before it is published anyway (each retry
/// costs another save and another copy, so the cap is small).
const MAX_RETRIES: u32 = 2;

/// Whether a finished copy that a save overlapped is thrown away and redone: the rule the
/// `retry_on_save` setting, the stale flag and the retry cap decide together.
fn should_retry(stale: bool, retry_on_save: bool, retries: u32) -> bool {
    stale && retry_on_save && retries < MAX_RETRIES
}

/// How many ticks a requested save may take before the copy starts without it. A save request
/// that never reports back must not stall the schedule for ever.
const SAVE_WAIT_TICKS: u64 = 200;

/// A finished copy of the world as a `.zip.part`, not yet published: the server tick that
/// receives it checks whether a save overlapped it before it renames the part (and prunes).
#[derive(Debug)]
struct Copy {
    part: PathBuf,
    final_path: PathBuf,
    /// The resolved backup folder the part was written in.
    dir: PathBuf,
    bytes: u64,
}

struct Report {
    name: String,
    bytes: u64,
    deleted: Vec<String>,
    /// Archives the disk cap removed from this folder (oldest first).
    cap_removed: Vec<String>,
    /// The folder's size after the prune, the cap and the removals.
    disk_bytes: u64,
    /// The folder is still over the cap with one archive left (upstream logs an error here).
    over_cap: bool,
    /// Published even though a save overlapped it (retries exhausted or off).
    stale: bool,
}

#[derive(Default)]
struct Core {
    cfg: Settings,
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
    /// The backup folder's size after the last publish.
    disk_bytes: u64,
    /// That folder is over `max_disk_size` and only one archive is left to delete.
    over_cap: bool,
    /// Settings lines the parser refused, shown by `/simplebackups status`.
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
            let cfg = Settings::default();
            let _ = files.write(&path, to_toml(&cfg).as_bytes());
            (cfg, Vec::new())
        }
        Err(_) => {
            let cfg = Settings::default();
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
    core.disk_bytes = 0;
    core.over_cap = false;
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
                    if should_retry(stale, core.cfg.retry_on_save, core.retries) {
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
                        if !report.cap_removed.is_empty() {
                            message.push_str(&format!(
                                "; removed {} over the disk cap",
                                report.cap_removed.len()
                            ));
                        }
                        if report.over_cap {
                            message.push_str(&format!(
                                "; still over the disk cap ({} of {})",
                                format_size_setting(report.disk_bytes),
                                format_size_setting(core.cfg.max_disk_bytes)
                            ));
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
    let (cfg, world, name, final_path, backups_prefix) = {
        let mut guard = core();
        let Some(core) = guard.as_mut() else {
            return;
        };
        let Some(world) = core.world.clone() else {
            core.in_flight = false;
            core.last_error = Some("no world open".to_owned());
            return;
        };
        let backup_dir = match owned_dir(&world, core.cfg.backups_dir()) {
            Ok(dir) => dir,
            Err(error) => {
                core.in_flight = false;
                core.last_error = Some(error);
                return;
            }
        };
        let backups_prefix = fs::canonicalize(&world)
            .ok()
            .and_then(|root| prefix_of(&root, &backup_dir));
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
        (core.cfg.clone(), world, name, final_path, backups_prefix)
    };

    let (tx, rx) = mpsc::channel();
    let worker_cfg = cfg.clone();
    let worker_final = final_path.clone();
    thread::spawn(move || {
        let result = run_backup(&world, &worker_final, &worker_cfg, backups_prefix);
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
fn run_backup(
    world: &Path,
    final_path: &Path,
    cfg: &Settings,
    backups_prefix: Option<String>,
) -> Result<Copy, String> {
    let world_name = world_name(world);
    let part = final_path.with_extension("zip.part");
    let result = (|| -> Result<u64, String> {
        // Reserve a **new** staging file. `File::create` follows a pre-existing
        // `.zip.part` (a stale file, or a symlink someone planted) and truncates
        // whatever it points at; removing the entry first removes a link itself, not
        // its target, and `create_new` then refuses a path that reappeared.
        match fs::symlink_metadata(&part) {
            Ok(meta) if meta.is_file() || meta.file_type().is_symlink() => {
                fs::remove_file(&part).map_err(|error| error.to_string())?;
            }
            Ok(_) => {
                return Err(format!("staging path {} is not a file", part.display()));
            }
            Err(_) => {}
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .map_err(|error| error.to_string())?;
        let mut zip = ZipWriter::new(BufWriter::new(file));
        walk(
            &mut zip,
            world,
            "",
            &world_name,
            cfg,
            backups_prefix.as_deref(),
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
        dir: final_path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
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
    let backups = copy.dir.clone();
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
    // Upstream enforces `maxDiskSize` after a good backup (`BackupThread.java:154-178`): the
    // oldest archives go while the folder is over the cap, and the last one always stays.
    let (cap_removed, disk_bytes, over_cap) =
        enforce_cap(&backups, &world_name, core.cfg.max_disk_bytes);
    core.disk_bytes = disk_bytes;
    core.over_cap = over_cap;
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
        cap_removed,
        disk_bytes,
        over_cap,
        stale,
    })
}

/// The folder's regular files as `(name, bytes)`. A folder and a symlink are not counted: the cap is
/// about bytes this folder holds, and following a link would charge the port for someone else's file.
fn folder_files(dir: &Path) -> Vec<(String, u64)> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| {
                    let meta = fs::symlink_metadata(entry.path()).ok()?;
                    meta.is_file()
                        .then(|| (entry.file_name().to_string_lossy().into_owned(), meta.len()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Delete this world's oldest archives while the backup folder is over `cap`, never the last one.
/// Returns what was removed, the folder size left, and whether it is still over the cap.
fn enforce_cap(dir: &Path, world: &str, cap: u64) -> (Vec<String>, u64, bool) {
    let plan = cap_removals(&folder_files(dir), world, cap);
    let mut removed = Vec::new();
    for name in &plan.remove {
        if fs::remove_file(dir.join(name)).is_ok() {
            removed.push(name.clone());
        }
    }
    let remaining: u64 = folder_files(dir).iter().map(|(_, bytes)| *bytes).sum();
    (removed, remaining, remaining > cap)
}

/// The backup folder, created if needed and verified to resolve **inside** the world.
/// `Settings::backups_dir` has already refused an absolute path and a parent component;
/// this checks what the filesystem actually resolves, because an existing symlink (or
/// junction) at the joined path can still point outside the world, and the worker
/// writes and prunes there.
fn owned_dir(world: &Path, relative: &str) -> Result<PathBuf, String> {
    let root = fs::canonicalize(world).map_err(|error| format!("world folder: {error}"))?;
    let dir = world.join(relative);
    fs::create_dir_all(&dir).map_err(|error| format!("backups folder: {error}"))?;
    let resolved = fs::canonicalize(&dir).map_err(|error| format!("backups folder: {error}"))?;
    if !resolved.starts_with(&root) {
        return Err(format!(
            "backups folder {} resolves outside the world {}",
            resolved.display(),
            root.display()
        ));
    }
    Ok(resolved)
}

/// The resolved folder's path relative to the world, in the walk's `/` spelling:
/// the subtree the copy must leave out, whatever spelling the config used.
fn prefix_of(root: &Path, dir: &Path) -> Option<String> {
    dir.strip_prefix(root)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
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
    cfg: &Settings,
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
        let backups = owned_dir(world, core.cfg.backups_dir()).map_or_else(
            |error| format!("<refused: {error}>"),
            |dir| dir.display().to_string(),
        );
        lines.push(format!("world={} backups={backups}", world.display()));
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
    // The disk cap is a config key of its own, so it gets a line of its own: the folder's size after
    // the last publish, the cap it is measured against, and whether it is still over it.
    lines.push(format!(
        "disk={} cap={} over={}",
        format_size_setting(core.disk_bytes),
        format_size_setting(core.cfg.max_disk_bytes),
        core.over_cap
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
    let dir = match owned_dir(world, core.cfg.backups_dir()) {
        Ok(dir) => dir,
        Err(error) => return vec![format!("Simple Backups: {error}")],
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A throw-away world folder with one file in it.
    fn scratch_world(tag: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let root = std::env::temp_dir().join(format!(
            "simplebackups-test-{tag}-{}-{unique}",
            std::process::id()
        ));
        let world = root.join("World");
        fs::create_dir_all(&world).expect("scratch world");
        fs::write(world.join("level.dat"), b"a world").expect("level.dat");
        world
    }

    fn cleanup(world: &Path) {
        if let Some(root) = world.parent() {
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn an_overlapped_copy_is_redone_only_within_the_rule() {
        assert!(should_retry(true, true, 0));
        assert!(should_retry(true, true, MAX_RETRIES - 1));
        assert!(
            !should_retry(false, true, 0),
            "a clean copy is published as it is"
        );
        assert!(
            !should_retry(true, false, 0),
            "retry_on_save=false keeps the stale copy"
        );
        assert!(
            !should_retry(true, true, MAX_RETRIES),
            "the cap stops the save/copy loop"
        );
    }

    #[test]
    fn a_planted_staging_symlink_is_not_followed_or_truncated() {
        let world = scratch_world("stage");
        let outside = world.parent().expect("a parent").join("outside.txt");
        fs::write(&outside, b"keep me").expect("outside file");
        let backups = world.join("backups");
        fs::create_dir_all(&backups).expect("backups folder");
        let final_path = backups.join("W_2026-01-01_00-00-00.zip");
        let part = final_path.with_extension("zip.part");
        symlink(&outside, &part).expect("planted symlink");

        let result = run_backup(
            &world,
            &final_path,
            &Settings::default(),
            Some("backups".to_owned()),
        );

        assert!(
            result.is_ok(),
            "the copy should succeed past a planted link: {result:?}"
        );
        assert_eq!(
            fs::read(&outside).expect("outside still readable"),
            b"keep me",
            "the link target must not be truncated"
        );
        let meta = fs::symlink_metadata(&part).expect("the part file");
        assert!(meta.is_file(), "the staging file must be a regular file");
        assert!(
            meta.len() > 0,
            "the staging file holds the copy, not the old link target"
        );
        cleanup(&world);
    }

    #[test]
    fn a_backups_folder_that_resolves_outside_the_world_is_refused() {
        let world = scratch_world("escape");
        let outside = world.parent().expect("a parent").join("elsewhere");
        fs::create_dir_all(&outside).expect("outside folder");
        symlink(&outside, world.join("backups")).expect("planted folder link");

        let refused = owned_dir(&world, "backups");
        assert!(
            refused.is_err(),
            "a backups folder that resolves outside the world must be refused, got {refused:?}"
        );
        cleanup(&world);
    }

    #[test]
    fn the_disk_cap_removes_the_oldest_archive_and_keeps_foreign_files() {
        let world = scratch_world("cap");
        let backups = world.join("backups");
        fs::create_dir_all(&backups).expect("backups folder");
        let older = backups.join("World_2026-01-01_00-00-00.zip");
        let newer = backups.join("World_2026-01-01_00-00-01.zip");
        fs::write(&older, vec![0u8; 400]).expect("older archive");
        fs::write(&newer, vec![0u8; 400]).expect("newer archive");
        let foreign = backups.join("someone-elses-map.zip");
        fs::write(&foreign, vec![0u8; 5_000]).expect("foreign archive");

        let (removed, remaining, over) = enforce_cap(&backups, "World", 1_000);

        assert_eq!(
            removed,
            ["World_2026-01-01_00-00-00.zip"],
            "only the oldest of our archives is removed"
        );
        assert_eq!(remaining, 5_400, "the folder still holds the foreign file");
        assert!(over, "one archive plus a foreign file is over 1 KB");
        assert!(!older.exists(), "the oldest archive is gone");
        assert!(newer.exists(), "the newest archive stays");
        assert!(foreign.exists(), "nothing foreign is deleted");
        cleanup(&world);
    }
}
