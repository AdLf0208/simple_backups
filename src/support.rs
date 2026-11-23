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
    ADOPTION_FILE, Adoption, Archive, MANIFEST_ENTRY, Settings, backup_file_name, cap_removals,
    copy_is_stale, due_by_millis, due_by_ticks, entry_name, format_adoptions, format_size,
    format_size_setting, is_adopted, manifest, manifest_identifies, parse, parse_adoptions,
    parse_archive, prune, should_skip, to_toml,
};
use crate::zip::{ZipWriter, entry_names, read_entry};

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
    /// Both retention passes were skipped because the published copy is stale.
    retention_skipped: bool,
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
                        if report.retention_skipped {
                            message.push_str("; retention skipped (a save overlapped this copy)");
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
        // The marker every archive carries: retention deletes only an archive whose manifest
        // names this port and this world, never a ZIP that merely matches the name prefix.
        let backup_name = final_path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let marker = manifest(&world_name, &backup_name);
        zip.add_bytes(MANIFEST_ENTRY, marker.as_bytes())
            .map_err(|error| error.to_string())?;
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
    let name = copy
        .final_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    fs::rename(&copy.part, &copy.final_path).map_err(|error| {
        let _ = fs::remove_file(&copy.part);
        error.to_string()
    })?;
    let cap = core.cfg.max_disk_bytes;
    let (deleted, cap_removed, disk_bytes, over_cap, retention_skipped) = if stale {
        // Both passes are skipped. This copy may hold a region file a save was replacing, so it
        // is not a restore point; deleting the previous confirmed one (by `keep` or by the cap)
        // would leave the stale copy as the only thing to restore from.
        let disk_bytes = folder_bytes(&backups);
        (Vec::new(), Vec::new(), disk_bytes, disk_bytes > cap, true)
    } else {
        let archives = owned_archives(&backups, &world_name);
        // The archive this call just wrote is never a removal candidate in either pass — it is
        // the backup this run promised, and an allocation or clock race must not delete it.
        let deleted = prune(&archives, core.cfg.keep, &name);
        for name in &deleted {
            let _ = fs::remove_file(backups.join(name));
        }
        // Upstream enforces `maxDiskSize` after a good backup (`BackupThread.java:154-178`): the
        // oldest archives go while the folder is over the cap, and the newest one always stays.
        let (cap_removed, disk_bytes, over_cap) = enforce_cap(&backups, &world_name, cap, &name);
        (deleted, cap_removed, disk_bytes, over_cap, false)
    };
    core.disk_bytes = disk_bytes;
    core.over_cap = over_cap;
    if stale {
        core.stale = true;
    }
    Ok(Report {
        name,
        bytes: copy.bytes,
        deleted,
        cap_removed,
        disk_bytes,
        over_cap,
        retention_skipped,
    })
}

/// Whether the archive at `path` carries this port's own marker: the `simplebackups.json` entry
/// with `format = simplebackups/1` and this world's name. A ZIP that merely matches the name
/// prefix, or has the world's shape, is not identified by it — see [`retainable`].
fn archive_is_ours(path: &Path, world: &str) -> bool {
    match read_entry(path, MANIFEST_ENTRY) {
        Ok(Some(bytes)) => manifest_identifies(&String::from_utf8_lossy(&bytes), world),
        Ok(None) | Err(_) => false,
    }
}

/// Whether retention may name this file as one of the port's archives.
///
/// Two things identity it: the marker every archive since 0.2.3 carries, or an **operator's
/// adoption** of a legacy archive (0.2.0–0.2.2 wrote no marker). The world-zip shape alone is
/// never enough: a hand-made export has the same name pattern and the same `World/level.dat`
/// inside it, so an operator has to say "this one is mine" with `/simplebackups adopt <file>`.
fn retainable(path: &Path, name: &str, bytes: u64, world: &str, adoptions: &[Adoption]) -> bool {
    if archive_is_ours(path, world) {
        return true;
    }
    is_adopted(adoptions, name, bytes) && legacy_world_zip(path, world)
}

fn legacy_world_zip(path: &Path, world: &str) -> bool {
    let Ok(names) = entry_names(path) else {
        return false;
    };
    let prefix = format!("{world}/");
    let level = format!("{prefix}level.dat");
    names.iter().any(|name| name == &level) && names.iter().all(|name| name.starts_with(&prefix))
}

/// The adoption journal in `dir`; a missing or unreadable journal adopts nothing.
fn read_adoptions(dir: &Path) -> Vec<Adoption> {
    fs::read_to_string(dir.join(ADOPTION_FILE))
        .map_or_else(|_| Vec::new(), |text| parse_adoptions(&text))
}

/// Replace the adoption journal atomically: a crash mid-write must not lose every adoption.
fn write_adoptions(dir: &Path, adoptions: &[Adoption]) -> io::Result<()> {
    let path = dir.join(ADOPTION_FILE);
    let staging = dir.join(format!("{ADOPTION_FILE}.tmp"));
    fs::write(&staging, format_adoptions(adoptions))?;
    fs::rename(&staging, &path)
}

/// An operator's `/simplebackups adopt <file>`: record a legacy archive (one written before the
/// manifest existed) as this world's, so retention may delete it.
///
/// The operator's word is the only identity a marker-less archive can have, so nothing here is
/// automatic: the name has to parse as this world's archive, the file has to be a world-shaped
/// ZIP, and the recorded size makes the adoption apply to this exact file only.
fn adopt_archive(dir: &Path, world: &str, requested: &str) -> Result<String, String> {
    let name = requested.trim();
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\t')
        || name == ".."
    {
        return Err("adopt needs one file name inside the backup folder".to_owned());
    }
    let path = dir.join(name);
    let meta =
        fs::symlink_metadata(&path).map_err(|_| format!("no {name} in {}", dir.display()))?;
    if !meta.is_file() {
        return Err(format!("{name} is not a regular file"));
    }
    let bytes = meta.len();
    if parse_archive(name, world, bytes).is_none() {
        return Err(format!(
            "{name} is not a <world>_<UTC>[-<generation>].zip name for this world"
        ));
    }
    if archive_is_ours(&path, world) {
        return Ok(format!(
            "{name} already carries this port's marker; nothing to adopt"
        ));
    }
    if !legacy_world_zip(&path, world) {
        return Err(format!(
            "{name} is not a {world} ZIP (it needs a {world}/level.dat)"
        ));
    }
    let mut adoptions = read_adoptions(dir);
    if is_adopted(&adoptions, name, bytes) {
        return Ok(format!("{name} is already adopted"));
    }
    adoptions.retain(|adoption| adoption.name != name);
    adoptions.push(Adoption {
        name: name.to_owned(),
        bytes,
    });
    adoptions.sort_by(|left, right| left.name.cmp(&right.name));
    write_adoptions(dir, &adoptions).map_err(|error| format!("adoption journal: {error}"))?;
    Ok(format!(
        "adopted {name} ({}); retention may now delete it",
        format_size(bytes)
    ))
}

/// The archives in `dir` retention may name: the port's marked archives and the legacy ones an
/// operator adopted.
fn owned_archives(dir: &Path, world: &str) -> Vec<Archive> {
    let adoptions = read_adoptions(dir);
    folder_files(dir)
        .into_iter()
        .filter(|(name, _)| name.ends_with(".zip"))
        .filter_map(|(name, bytes)| {
            if !retainable(&dir.join(&name), &name, bytes, world, &adoptions) {
                return None;
            }
            parse_archive(&name, world, bytes)
        })
        .collect()
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

/// The folder's regular-file bytes.
fn folder_bytes(dir: &Path) -> u64 {
    folder_files(dir).iter().map(|(_, bytes)| *bytes).sum()
}

/// Delete the oldest **identified** archives while the backup folder is over `cap`, never the
/// archive just published and never the last one left. Returns what was removed, the folder size
/// left, and whether it is still over.
fn enforce_cap(dir: &Path, world: &str, cap: u64, protected: &str) -> (Vec<String>, u64, bool) {
    let files = folder_files(dir);
    let archives = owned_archives(dir, world);
    let plan = cap_removals(&files, &archives, cap, protected);
    let mut removed = Vec::new();
    for name in &plan.remove {
        if fs::remove_file(dir.join(name)).is_ok() {
            removed.push(name.clone());
        }
    }
    let remaining = folder_bytes(dir);
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
    // An operator-level action: it is the *only* way a marker-less archive becomes deletable.
    commands.push(Command::op("simplebackups adopt <file:text>", 2));
}

/// The adopt command's body: the open world's backup folder and the file name an operator named.
fn adopt_file(file: &str) -> Result<String, String> {
    let (world, cfg) = {
        let guard = core();
        let core = guard
            .as_ref()
            .ok_or_else(|| "Simple Backups is not initialised.".to_owned())?;
        let world = core
            .world
            .clone()
            .ok_or_else(|| "No world is open.".to_owned())?;
        (world, core.cfg.clone())
    };
    let dir = owned_dir(&world, cfg.backups_dir())?;
    adopt_archive(&dir, &world_name(&world), file)
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
        ["simplebackups", "adopt"] => {
            let file = match event.command.argument("file") {
                Some(mod_api::CommandArgumentValue::String(text)) => text.clone(),
                _ => String::new(),
            };
            let reply = adopt_file(&file).unwrap_or_else(|error| format!("Adopt failed: {error}"));
            let _ = host.commands().feedback(Text::literal(reply));
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
    // the last publish, the cap it is measured against, and whether it is still over it. The
    // adoption count is the number of legacy archives an operator made deletable.
    let adopted = core
        .world
        .as_ref()
        .and_then(|world| owned_dir(world, core.cfg.backups_dir()).ok())
        .map_or(0, |dir| read_adoptions(&dir).len());
    lines.push(format!(
        "disk={} cap={} over={} adopted={adopted}",
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

    /// Settings with the two retention bounds a scene needs.
    fn settings_with(keep: usize, cap: u64) -> Settings {
        Settings {
            keep,
            max_disk_bytes: cap,
            ..Settings::default()
        }
    }

    /// A `Core` as the tick would hold it for one open world.
    fn core_with(world: &Path, cfg: Settings) -> Core {
        Core {
            cfg,
            world: Some(world.to_path_buf()),
            ..Core::default()
        }
    }

    /// A plain ZIP with the given entries and **no** port marker: what a player, an old
    /// version of another tool or a hand-made archive would leave behind.
    fn plain_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut writer = crate::zip::ZipWriter::new(fs::File::create(path).expect("create zip"));
        let source = path.with_extension("entry-src");
        for (name, bytes) in entries {
            fs::write(&source, bytes).expect("entry source");
            writer.add_file(name, &source).expect("add_file");
        }
        let _ = fs::remove_file(&source);
        let file = writer.finish().expect("finish");
        file.sync_all().expect("sync");
    }

    fn archive_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("read backups")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".zip"))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_stale_copy_skips_both_retention_passes() {
        let world = scratch_world("stale-skip");
        let backups = world.join("backups");
        fs::create_dir_all(&backups).expect("backups folder");
        // keep = 1 and a 1 B cap both want to delete the moment they run.
        let cfg = settings_with(1, 1);
        let first = run_backup(
            &world,
            &backups.join("World_2026-01-01_00-00-00.zip"),
            &cfg,
            Some("backups".to_owned()),
        )
        .expect("the first copy");
        publish(&mut core_with(&world, cfg.clone()), &first, false).expect("publish first");
        let second = run_backup(
            &world,
            &backups.join("World_2026-01-01_00-00-01.zip"),
            &cfg,
            Some("backups".to_owned()),
        )
        .expect("the second copy");
        publish(&mut core_with(&world, cfg.clone()), &second, false).expect("publish second");
        assert_eq!(
            archive_names(&backups),
            ["World_2026-01-01_00-00-01.zip"],
            "the scene starts with one confirmed restore point"
        );

        let stale = run_backup(
            &world,
            &backups.join("World_2026-01-01_00-00-02.zip"),
            &cfg,
            Some("backups".to_owned()),
        )
        .expect("the stale copy");
        let report = publish(&mut core_with(&world, cfg), &stale, true).expect("publish stale");

        assert!(
            report.deleted.is_empty(),
            "keep must not run after a stale copy: {:?}",
            report.deleted
        );
        assert!(
            report.cap_removed.is_empty(),
            "the disk cap must not run after a stale copy: {:?}",
            report.cap_removed
        );
        assert_eq!(
            archive_names(&backups),
            [
                "World_2026-01-01_00-00-01.zip",
                "World_2026-01-01_00-00-02.zip"
            ],
            "the previous restore point survives beside the stale copy"
        );
        cleanup(&world);
    }

    #[test]
    fn a_foreign_zip_named_like_a_backup_survives_retention() {
        let world = scratch_world("foreign-zip");
        let backups = world.join("backups");
        fs::create_dir_all(&backups).expect("backups folder");
        // A hand-made archive with the world prefix and an older name than any real backup:
        // the name pattern alone would make it the first thing both passes delete.
        let hand_made = backups.join("World_1999-01-01_00-00-00.zip");
        plain_zip(&hand_made, &[("my-notes.txt", b"do not delete")]);
        let cfg = settings_with(1, 1);
        let old = run_backup(
            &world,
            &backups.join("World_2026-01-01_00-00-00.zip"),
            &cfg,
            Some("backups".to_owned()),
        )
        .expect("the old copy");
        publish(&mut core_with(&world, cfg.clone()), &old, false).expect("publish old");
        let new = run_backup(
            &world,
            &backups.join("World_2026-01-01_00-00-01.zip"),
            &cfg,
            Some("backups".to_owned()),
        )
        .expect("the new copy");
        publish(&mut core_with(&world, cfg), &new, false).expect("publish new");

        assert!(
            hand_made.exists(),
            "a zip the mod did not write is not a backup and must survive"
        );
        assert!(
            !backups.join("World_2026-01-01_00-00-00.zip").exists(),
            "the older archive of the same age is removed (the differential)"
        );
        assert!(
            backups.join("World_2026-01-01_00-00-01.zip").exists(),
            "the newest restore point survives"
        );
        cleanup(&world);
    }

    #[test]
    fn a_manual_export_with_the_world_shape_needs_an_operator_to_become_deletable() {
        let world = scratch_world("manual-export");
        let backups = world.join("backups");
        fs::create_dir_all(&backups).expect("backups folder");
        // An export a player made by hand: the world prefix, the oldest name, and the world's
        // *own* files (`World/level.dat`, a region file), so the world-zip shape cannot tell it
        // from an archive this port wrote before it carried a manifest.
        let export = backups.join("World_1999-01-01_00-00-00.zip");
        plain_zip(
            &export,
            &[
                ("World/level.dat", b"a world"),
                ("World/region/r.0.0.mca", b"chunk"),
            ],
        );

        let cfg = settings_with(1, 1);
        let first = run_backup(
            &world,
            &backups.join("World_2026-01-01_00-00-00.zip"),
            &cfg,
            Some("backups".to_owned()),
        )
        .expect("the first copy");
        publish(&mut core_with(&world, cfg.clone()), &first, false).expect("publish first");
        let second = run_backup(
            &world,
            &backups.join("World_2026-01-01_00-00-01.zip"),
            &cfg,
            Some("backups".to_owned()),
        )
        .expect("the second copy");
        publish(&mut core_with(&world, cfg), &second, false).expect("publish second");

        assert!(
            export.exists(),
            "an archive no operator adopted must survive both retention passes, whatever its shape"
        );
        cleanup(&world);
    }

    #[test]
    fn the_archive_just_written_survives_both_retention_passes() {
        let world = scratch_world("just-written");
        let backups = world.join("backups");
        fs::create_dir_all(&backups).expect("backups folder");
        // A higher generation of the same second already exists, so an archive allocated at
        // the base name would sort behind it and be the first thing retention deletes.
        let seed = run_backup(
            &world,
            &backups.join("World_2026-01-01_00-00-00-9.zip"),
            &Settings::default(),
            Some("backups".to_owned()),
        )
        .expect("the seed archive");
        publish(
            &mut core_with(&world, settings_with(3, u64::MAX)),
            &seed,
            false,
        )
        .expect("publish the seed");
        let fresh = run_backup(
            &world,
            &backups.join("World_2026-01-01_00-00-00.zip"),
            &Settings::default(),
            Some("backups".to_owned()),
        )
        .expect("the fresh copy");
        // keep = 1 and a 1 B cap both want to delete the oldest archive of that second.
        let report = publish(&mut core_with(&world, settings_with(1, 1)), &fresh, false)
            .expect("publish the fresh copy");

        assert!(
            backups.join("World_2026-01-01_00-00-00.zip").exists(),
            "the archive just written is never deleted by either pass (keep removed {:?}, cap removed {:?})",
            report.deleted,
            report.cap_removed
        );
        cleanup(&world);
    }

    #[test]
    fn an_adopted_legacy_zip_becomes_deletable() {
        let world = scratch_world("adopt");
        let backups = world.join("backups");
        fs::create_dir_all(&backups).expect("backups folder");
        let export_name = "World_1999-01-01_00-00-00.zip";
        let export = backups.join(export_name);
        plain_zip(
            &export,
            &[
                ("World/level.dat", b"a world"),
                ("World/region/r.0.0.mca", b"chunk"),
            ],
        );

        let cfg = settings_with(1, 1);
        let first = run_backup(
            &world,
            &backups.join("World_2026-01-01_00-00-00.zip"),
            &cfg,
            Some("backups".to_owned()),
        )
        .expect("the first copy");
        publish(&mut core_with(&world, cfg.clone()), &first, false).expect("publish first");
        assert!(
            export.exists(),
            "an export nobody adopted survives retention"
        );

        // Only an archive of this world may be adopted, and only a world ZIP at that.
        assert!(adopt_archive(&backups, "World", "notes.txt").is_err());
        assert!(adopt_archive(&backups, "World", "../outside.zip").is_err());
        let not_a_world = "World_2000-01-01_00-00-00.zip";
        plain_zip(&backups.join(not_a_world), &[("readme.txt", b"x")]);
        assert!(
            adopt_archive(&backups, "World", not_a_world).is_err(),
            "a ZIP without the world in it is not adoptable as a backup"
        );

        let reply = adopt_archive(&backups, "World", export_name).expect("adopt the export");
        assert!(reply.contains("adopted"), "{reply}");

        // The operator's word is what makes the next publish delete it.
        let second = run_backup(
            &world,
            &backups.join("World_2026-01-01_00-00-01.zip"),
            &cfg,
            Some("backups".to_owned()),
        )
        .expect("the second copy");
        publish(&mut core_with(&world, cfg), &second, false).expect("publish second");
        assert!(
            !export.exists(),
            "an adopted archive is this world's and may be pruned"
        );
        assert!(
            backups.join("World_2026-01-01_00-00-01.zip").exists(),
            "the newest backup survives"
        );
        cleanup(&world);
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
        let cfg = Settings::default();
        let older_name = "World_2026-01-01_00-00-00.zip";
        let newer_name = "World_2026-01-01_00-00-01.zip";
        let older = run_backup(
            &world,
            &backups.join(older_name),
            &cfg,
            Some("backups".to_owned()),
        )
        .expect("older archive");
        publish(&mut core_with(&world, cfg.clone()), &older, false).expect("publish older");
        let newer = run_backup(
            &world,
            &backups.join(newer_name),
            &cfg,
            Some("backups".to_owned()),
        )
        .expect("newer archive");
        publish(&mut core_with(&world, cfg.clone()), &newer, false).expect("publish newer");
        let foreign = backups.join("someone-elses-map.zip");
        plain_zip(&foreign, &[("map.dat", &[0u8; 100])]);

        let older_bytes = fs::metadata(backups.join(older_name)).expect("older").len();
        let newer_bytes = fs::metadata(backups.join(newer_name)).expect("newer").len();
        let foreign_bytes = fs::metadata(&foreign).expect("foreign").len();
        // Exactly the newest archive plus the foreign file fits: the cap has to remove the older.
        let cap = newer_bytes + foreign_bytes;
        assert!(cap > older_bytes, "the scene needs a removal");

        let (removed, remaining, over) = enforce_cap(&backups, "World", cap, "");

        assert_eq!(removed, [older_name], "only the oldest archive is removed");
        assert_eq!(remaining, cap, "the folder still holds the foreign file");
        assert!(!over, "the cap is satisfied");
        assert!(
            !backups.join(older_name).exists(),
            "the oldest archive is gone"
        );
        assert!(
            backups.join(newer_name).exists(),
            "the newest archive stays"
        );
        assert!(foreign.exists(), "nothing foreign is deleted");
        cleanup(&world);
    }

    #[test]
    fn an_archive_identifies_itself_not_by_its_name() {
        let world = scratch_world("identity");
        let backups = world.join("backups");
        fs::create_dir_all(&backups).expect("backups folder");
        let cfg = Settings::default();
        let ours_name = "World_2026-01-01_00-00-00.zip";
        let ours = run_backup(
            &world,
            &backups.join(ours_name),
            &cfg,
            Some("backups".to_owned()),
        )
        .expect("our copy");
        publish(&mut core_with(&world, cfg.clone()), &ours, false).expect("publish ours");
        assert!(
            archive_is_ours(&backups.join(ours_name), "World"),
            "a copy the port wrote carries its marker"
        );

        // Same writer, same name pattern, no marker: someone else's archive.
        let foreign = backups.join("World_2026-01-01_00-00-09.zip");
        plain_zip(&foreign, &[("notes.txt", b"hello")]);
        assert!(
            !archive_is_ours(&foreign, "World"),
            "a world prefix and a ZIP are not an identity"
        );

        // A marker for another world does not identify this world's archive either.
        let other = backups.join("World_2026-01-01_00-00-10.zip");
        let mut writer = crate::zip::ZipWriter::new(fs::File::create(&other).expect("create"));
        writer
            .add_bytes(
                crate::runtime::MANIFEST_ENTRY,
                crate::runtime::manifest("Other", "x.zip").as_bytes(),
            )
            .expect("manifest entry");
        writer.finish().expect("finish").sync_all().expect("sync");
        assert!(
            !archive_is_ours(&other, "World"),
            "another world's manifest is not this world's archive"
        );

        // An archive from before the marker (0.2.0-0.2.2) has no identity of its own: the world
        // shape it has is exactly what a hand-made export reproduces. An operator's adoption is
        // the identity, and only then may retention name it.
        let legacy = backups.join("World_2026-01-01_00-00-11.zip");
        plain_zip(
            &legacy,
            &[
                ("World/level.dat", b"a world"),
                ("World/region/r.0.0.mca", b"chunk"),
            ],
        );
        assert!(
            !archive_is_ours(&legacy, "World"),
            "a marker-less archive is not identified by its shape"
        );
        let legacy_bytes = fs::metadata(&legacy).expect("legacy archive").len();
        let adoptions = [Adoption {
            name: "World_2026-01-01_00-00-11.zip".to_owned(),
            bytes: legacy_bytes,
        }];
        assert!(
            retainable(
                &legacy,
                "World_2026-01-01_00-00-11.zip",
                legacy_bytes,
                "World",
                &adoptions
            ),
            "an operator's adoption is the identity a legacy archive can have"
        );
        assert!(
            !retainable(
                &legacy,
                "World_2026-01-01_00-00-11.zip",
                legacy_bytes + 1,
                "World",
                &adoptions
            ),
            "a file replaced since the adoption is not adopted"
        );
        cleanup(&world);
    }
}
