// Simple Backups for Horizon — the backup policy (pure).
//
// Ported from Simple Backups 1.21-4.0.30 (ChaoticTrials/SimpleBackups,
// Apache-2.0) for Minecraft Java 1.21.1 NeoForge. Upstream source read for
// behaviour only; this is a clean-room Rust rewrite.
//
// Slice 1: config, due time (ticks or wall clock), UTC zip names, retention,
// entry names and skip rules. The glue does the save, the zip, and the
// commands.

use std::path::{Path, PathBuf};

/// Upstream-compatible configuration (`config/simplebackups.toml`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub interval_minutes: u64,
    pub interval_ticks: u64,
    pub keep: usize,
    pub compression_level: i32,
    pub save_before_backup: bool,
    /// Retry the copy when a save cycle ran while it was in flight.
    pub retry_on_save: bool,
    pub broadcast: bool,
    pub backups_dir: String,
    /// The whole backup folder may not exceed this many bytes after a good backup.
    pub max_disk_bytes: u64,
    pub ignored_paths: Vec<String>,
    pub ignored_files: Vec<String>,
    /// Full backups, or an incremental chain (upstream's experimental backup type).
    pub backup_type: BackupType,
    /// How long an incremental chain runs before the next base is forced (minutes).
    pub full_backup_timer_minutes: u64,
    /// Back up on the schedule while nobody is online (upstream `noPlayerBackups`).
    pub no_player_backups: bool,
    /// How many empty-server backups per period until a player joins (upstream
    /// `noPlayerBackupCount`; 0 disables them).
    pub no_player_backup_count: u64,
    /// Empty-server interval in minutes (upstream `noPlayerBackupTimer`; 0 means
    /// the same as `interval_minutes`).
    pub no_player_backup_timer_minutes: u64,
    /// Append the game's `logs/latest.log` to every archive as `backup.log`
    /// (upstream `captureLatestLog`, default true).
    pub capture_latest_log: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: true,
            interval_minutes: 120,
            interval_ticks: 0,
            keep: 10,
            compression_level: -1,
            save_before_backup: true,
            retry_on_save: true,
            broadcast: true,
            backups_dir: "backups".to_owned(),
            max_disk_bytes: DEFAULT_MAX_DISK_BYTES,
            ignored_paths: Vec::new(),
            ignored_files: Vec::new(),
            // Upstream ships full backups by default; chains are an experimental opt-in.
            backup_type: BackupType::Full,
            full_backup_timer_minutes: DEFAULT_FULL_BACKUP_TIMER_MINUTES,
            no_player_backups: false,
            no_player_backup_count: 1,
            // Upstream `noPlayerBackupTimer` defaults to 0 (use `timer`).
            no_player_backup_timer_minutes: 0,
            // Upstream `captureLatestLog` defaults to true.
            capture_latest_log: true,
        }
    }
}

impl Settings {
    /// The backup folder, treating an empty setting as the default. The parser
    /// normalises it through [`safe_backups_dir`], so it is always a relative path
    /// with no `.`/`..` component.
    pub fn backups_dir(&self) -> &str {
        if self.backups_dir.trim().is_empty() {
            "backups"
        } else {
            &self.backups_dir
        }
    }
}

/// A configured backup folder as a safe **world-relative** path: an absolute path
/// and a parent component are refused, `.`/empty components are dropped (so
/// `./backups` and `backups/` are the same folder), and nothing may resolve to the
/// world itself. The glue additionally verifies the resolved path, because an
/// existing symlink at that name can still point outside the world.
#[must_use]
pub fn safe_backups_dir(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let raw = raw.replace('\\', "/");
    if raw.starts_with('/') {
        return None;
    }
    let mut parts = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            other => parts.push(other),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Parse the simple `key = value` config used by the port. Bad values keep
/// their defaults and are reported; unknown keys are reported too.
pub fn parse(input: &str) -> (Settings, Vec<String>) {
    let mut config = Settings::default();
    let mut warnings = Vec::new();
    for raw in input.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            warnings.push(format!("ignored line: {line}"));
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "enabled" => match parse_bool(value) {
                Some(parsed) => config.enabled = parsed,
                None => warnings.push(format!("enabled: not a boolean: {value}")),
            },
            "interval_minutes" => match value.parse::<u64>() {
                Ok(parsed) => config.interval_minutes = parsed,
                Err(_) => warnings.push(format!("interval_minutes: not a number: {value}")),
            },
            "interval_ticks" => match value.parse::<u64>() {
                Ok(parsed) => config.interval_ticks = parsed,
                Err(_) => warnings.push(format!("interval_ticks: not a number: {value}")),
            },
            "keep" => match value.parse::<usize>() {
                Ok(0) => {
                    config.keep = 1;
                    warnings.push("keep: clamped to 1".to_owned());
                }
                Ok(parsed) => config.keep = parsed,
                Err(_) => warnings.push(format!("keep: not a number: {value}")),
            },
            "compression_level" => match value.parse::<i32>() {
                Ok(parsed) if (-1..=9).contains(&parsed) => config.compression_level = parsed,
                Ok(parsed) => {
                    config.compression_level = parsed.clamp(-1, 9);
                    warnings.push(format!("compression_level: clamped from {parsed}"));
                }
                Err(_) => warnings.push(format!("compression_level: not a number: {value}")),
            },
            "save_before_backup" => match parse_bool(value) {
                Some(parsed) => config.save_before_backup = parsed,
                None => warnings.push(format!("save_before_backup: not a boolean: {value}")),
            },
            "retry_on_save" => match parse_bool(value) {
                Some(parsed) => config.retry_on_save = parsed,
                None => warnings.push(format!("retry_on_save: not a boolean: {value}")),
            },
            "broadcast" => match parse_bool(value) {
                Some(parsed) => config.broadcast = parsed,
                None => warnings.push(format!("broadcast: not a boolean: {value}")),
            },
            "backups_dir" => match strip_quotes(value).and_then(safe_backups_dir) {
                Some(parsed) => config.backups_dir = parsed,
                None => warnings.push(format!(
                    "backups_dir: not a safe world-relative path: {value}"
                )),
            },
            "max_disk_size" => match parse_size(value) {
                Some(parsed) => config.max_disk_bytes = parsed,
                None => warnings.push(format!(
                    "max_disk_size: not a size such as \"25 GB\": {value}"
                )),
            },
            "ignored_paths" => match parse_array(value) {
                Some(parsed) => config.ignored_paths = parsed,
                None => warnings.push(format!("ignored_paths: not an array: {value}")),
            },
            "ignored_files" => match parse_array(value) {
                Some(parsed) => config.ignored_files = parsed,
                None => warnings.push(format!("ignored_files: not an array: {value}")),
            },
            "backup_type" => match parse_backup_type(value) {
                Some(parsed) => config.backup_type = parsed,
                None => warnings.push(format!(
                    "backup_type: not full|incremental|differential: {value}"
                )),
            },
            "full_backup_timer" => match value.parse::<u64>() {
                Ok(parsed) => config.full_backup_timer_minutes = parsed,
                Err(_) => warnings.push(format!("full_backup_timer: not a number: {value}")),
            },
            "no_player_backups" => match parse_bool(value) {
                Some(parsed) => config.no_player_backups = parsed,
                None => warnings.push(format!("no_player_backups: not a boolean: {value}")),
            },
            "no_player_backup_count" => match value.parse::<u64>() {
                Ok(parsed) => config.no_player_backup_count = parsed,
                Err(_) => warnings.push(format!("no_player_backup_count: not a number: {value}")),
            },
            "no_player_backup_timer" => match value.parse::<u64>() {
                Ok(parsed) => config.no_player_backup_timer_minutes = parsed,
                Err(_) => warnings.push(format!("no_player_backup_timer: not a number: {value}")),
            },
            "capture_latest_log" => match parse_bool(value) {
                Some(parsed) => config.capture_latest_log = parsed,
                None => warnings.push(format!("capture_latest_log: not a boolean: {value}")),
            },
            other => warnings.push(format!("unknown key: {other}")),
        }
    }
    (config, warnings)
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn strip_quotes(value: &str) -> Option<&str> {
    let value = value.trim();
    if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return Some(inner);
    }
    None
}

fn parse_array(value: &str) -> Option<Vec<String>> {
    let inner = value.trim().strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for item in inner.split(',') {
        let item = item.trim();
        let item = strip_quotes(item)?;
        out.push(item.to_owned());
    }
    Some(out)
}

/// The default file written when no config exists.
pub fn to_toml(config: &Settings) -> String {
    let mut out = String::new();
    out.push_str("# Simple Backups — world backups (Apache-2.0 port)\n");
    out.push_str(&format!("enabled = {}\n", config.enabled));
    out.push_str(&format!("interval_minutes = {}\n", config.interval_minutes));
    out.push_str(&format!("interval_ticks = {}\n", config.interval_ticks));
    out.push_str(&format!("keep = {}\n", config.keep));
    out.push_str(&format!(
        "compression_level = {}\n",
        config.compression_level
    ));
    out.push_str(&format!(
        "save_before_backup = {}\n",
        config.save_before_backup
    ));
    out.push_str(&format!("retry_on_save = {}\n", config.retry_on_save));
    out.push_str(&format!("broadcast = {}\n", config.broadcast));
    out.push_str(&format!("backups_dir = \"{}\"\n", config.backups_dir()));
    out.push_str(&format!(
        "max_disk_size = \"{}\"\n",
        format_size_setting(config.max_disk_bytes)
    ));
    out.push_str(&format!(
        "backup_type = \"{}\"\n",
        config.backup_type.as_str()
    ));
    out.push_str(&format!(
        "full_backup_timer = {}\n",
        config.full_backup_timer_minutes
    ));
    out.push_str(&format!(
        "no_player_backups = {}\n",
        config.no_player_backups
    ));
    out.push_str(&format!(
        "no_player_backup_count = {}\n",
        config.no_player_backup_count
    ));
    out.push_str(&format!(
        "no_player_backup_timer = {}\n",
        config.no_player_backup_timer_minutes
    ));
    out.push_str(&format!(
        "capture_latest_log = {}\n",
        config.capture_latest_log
    ));
    out
}

/// Whether a tick-mode backup is due (`interval_ticks > 0`).
pub fn due_by_ticks(last_tick: u64, now_tick: u64, interval: u64) -> bool {
    interval > 0 && now_tick.saturating_sub(last_tick) >= interval
}

/// Whether a wall-clock backup is due (0 minutes disables the schedule).
pub fn due_by_millis(last_ms: u64, now_ms: u64, interval_minutes: u64) -> bool {
    interval_minutes > 0
        && now_ms.saturating_sub(last_ms) >= interval_minutes.saturating_mul(60_000)
}

/// The schedule interval, in minutes, for the current player presence. Upstream's
/// `CommonConfig.getTimer(arePlayersOnline)` (`CommonConfig.java:151-157`) takes
/// `noPlayerBackupTimer`, replaces it with `timer` when it is 0, and then picks
/// `timer` whenever a player is online: `(online ? timer : i) * 60 * 1000`. This
/// returns the minutes; the caller's `due_by_millis` supplies the × 60 000.
#[must_use]
pub fn empty_period_interval(players_online: bool, main_minutes: u64, empty_minutes: u64) -> u64 {
    if players_online || empty_minutes == 0 {
        main_minutes
    } else {
        empty_minutes
    }
}

/// The persisted empty-server counter, stored beside the world. Upstream keeps
/// `backupsSinceLastPlayerJoined` in its `BackupData` SavedData, so it survives a restart.
pub const NO_PLAYER_FILE: &str = "simplebackups-no-player.txt";

/// Whether a scheduled backup may start with the given player presence and empty-server policy.
/// Upstream's player-presence rule wraps `shouldRunBackup`: the enabled/paused/due gate still
/// applies, and with nobody online the schedule only runs when `no_player_backups` is on and the
/// per-period count has not been used up.
pub fn scheduled_backup_permitted(
    enabled: bool,
    paused: bool,
    players_online: bool,
    no_player_backups: bool,
    no_player_backup_count: u64,
    backups_since_player: u64,
) -> bool {
    if !schedule_allowed(enabled, paused) {
        return false;
    }
    players_online
        || (no_player_backups
            && no_player_backup_count != 0
            && backups_since_player < no_player_backup_count)
}

/// The one line this port writes for the persisted empty-server counter.
pub fn format_no_player_count(count: u64) -> String {
    format!("backups = {count}\n")
}

/// Read the persisted counter. Only an exact line this port writes counts: an absent,
/// malformed or hand-edited file starts a fresh empty period (zero backups).
pub fn parse_no_player_count(text: &str) -> Option<u64> {
    text.trim().strip_prefix("backups = ")?.parse::<u64>().ok()
}

/// The persisted pause flag, stored beside the world. Upstream keeps `paused` in its
/// `BackupData` `SavedData` (NBT boolean `paused`, `setDirty()` on change); a native mod has no
/// SavedData API, so the same bit lives in this file and is read when the world opens.
pub const PAUSE_FILE: &str = "simplebackups-paused.txt";

/// Whether the **schedule** may start a backup. Upstream's `shouldRunBackup` returns false while
/// paused; a manual `backup` command calls `createBackup` directly and ignores both `enabled`
/// and `paused`.
pub fn schedule_allowed(enabled: bool, paused: bool) -> bool {
    enabled && !paused
}

/// The one line this port writes for the persisted flag.
pub fn format_paused(paused: bool) -> String {
    format!("paused = {paused}\n")
}

/// Read the persisted flag. Only an exact line this port writes counts as paused: an absent,
/// malformed or hand-edited file leaves the schedule running.
pub fn parse_paused(text: &str) -> Option<bool> {
    match text.trim() {
        "paused = true" => Some(true),
        "paused = false" => Some(false),
        _ => None,
    }
}

/// The persisted schedule counter, stored beside the world. Upstream keeps `lastSaved` in its
/// `BackupData` `SavedData` so a restart does not reset the interval; a native mod has no
/// SavedData API, so the ticks elapsed since the last backup live in this file. `ensure_world`
/// reads it when the world opens, and the save/close hooks write it back.
pub const SCHEDULE_FILE: &str = "simplebackups-schedule.txt";

/// Progress toward the next scheduled backup: ticks since the last one started. The persisted
/// file stores this value, not the absolute tick, so a new session can resume from it.
pub fn schedule_progress(ticks: u64, last_tick: u64) -> u64 {
    ticks.saturating_sub(last_tick)
}

/// The one line this port writes for the persisted schedule counter.
pub fn format_schedule_ticks(ticks: u64) -> String {
    format!("ticks = {ticks}\n")
}

/// Read the persisted tick counter. Only an exact line this port writes counts: an absent,
/// malformed or hand-edited file means a fresh schedule (zero ticks toward the next backup).
pub fn parse_schedule_ticks(text: &str) -> Option<u64> {
    text.trim().strip_prefix("ticks = ")?.parse::<u64>().ok()
}

/// Whether a copy that started at `start_seq` overlapped a save: the world's
/// save notification counter moved while the zip was being written, so a
/// region file may be half-replaced inside it. Such a copy is thrown away and
/// redone rather than published.
pub fn copy_is_stale(start_seq: u64, current_seq: u64) -> bool {
    start_seq != current_seq
}

/// `<world>_<YYYY-MM-DD_HH-MM-SS>` in UTC (`civil_from_days`, no chrono).
pub fn backup_file_stem(world: &str, unix_secs: i64) -> String {
    let (year, month, day, hour, minute, second) = utc_parts(unix_secs);
    format!("{world}_{year:04}-{month:02}-{day:02}_{hour:02}-{minute:02}-{second:02}")
}

/// The next free name for this second: the base `<stem>.zip` when nothing of that second
/// exists, else **one above the highest suffix present** (`-1`, `-2` …).
///
/// Taking the lowest free suffix instead would reuse a number retention pruned and put the
/// new archive *behind* an older one of the same second — the very pass that runs after the
/// publish would then delete the backup it just wrote.
pub fn backup_file_name(world: &str, unix_secs: i64, taken: &[String]) -> String {
    let stem = backup_file_stem(world, unix_secs);
    let highest = taken
        .iter()
        .filter_map(|candidate| generation_of(candidate, &stem))
        .max();
    match highest {
        Some(generation) => format!("{stem}-{}.zip", generation.saturating_add(1)),
        None => format!("{stem}.zip"),
    }
}

/// The generation a name carries for `stem`: `0` for `<stem>.zip`, `n` for `<stem>-n.zip`,
/// and `None` for any other name (a `.part`, another second, another world, a foreign file).
fn generation_of(candidate: &str, stem: &str) -> Option<u64> {
    let rest = candidate.strip_prefix(stem)?.strip_suffix(".zip")?;
    if rest.is_empty() {
        return Some(0);
    }
    let digits = rest.strip_prefix('-')?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn utc_parts(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = (rem / 3_600) as u32;
    let minute = ((rem % 3_600) / 60) as u32;
    let second = (rem % 60) as u32;
    (year, month, day, hour, minute, second)
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (year + i64::from(month <= 2), month as u32, day as u32)
}

/// The marker entry every archive this port writes carries, so retention can tell its own
/// backups from any other ZIP that happens to sit in the folder (a hand-made archive with the
/// world prefix, another tool's backup, a map a player kept). `zip::read_entry` reads it.
pub const MANIFEST_ENTRY: &str = "simplebackups.json";

/// The manifest's format tag; a different value is not this port's archive.
pub const MANIFEST_FORMAT: &str = "simplebackups/1";

/// One archive **this port wrote**, as its name and bytes identify it. Retention never touches
/// an archive it cannot build one of these for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Archive {
    pub name: String,
    /// The UTC stamp (`YYYY-MM-DD_HH-MM-SS`) the name carries.
    pub stamp: String,
    /// The collision suffix: 0 for the base name, then `-1`, `-2` … (numeric, not text).
    pub generation: u64,
    pub bytes: u64,
}

/// Parse `<world>_<YYYY-MM-DD_HH-MM-SS>[-<generation>].zip`. The generation is numeric, so
/// `-10` follows `-9` and the base name is generation 0.
///
/// The name comes from the filesystem, so it is untrusted: the stamp is taken with a
/// **char-boundary-checked** slice (`str::get`), not `split_at(19)`, because a name with a
/// multibyte character at byte 19 would otherwise panic on the server tick.
#[must_use]
pub fn parse_archive(name: &str, world: &str, bytes: u64) -> Option<Archive> {
    let rest = name
        .strip_prefix(world)?
        .strip_prefix('_')?
        .strip_suffix(".zip")?;
    if rest.len() < 19 {
        return None;
    }
    let stamp = rest.get(..19)?;
    let suffix = rest.get(19..)?;
    if !is_stamp(stamp) {
        return None;
    }
    let generation = if suffix.is_empty() {
        0
    } else {
        let digits = suffix.strip_prefix('-')?;
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok()?
    };
    Some(Archive {
        name: name.to_owned(),
        stamp: stamp.to_owned(),
        generation,
        bytes,
    })
}

fn is_stamp(stamp: &str) -> bool {
    let bytes = stamp.as_bytes();
    bytes.len() == 19
        && [4, 7].iter().all(|at| bytes[*at] == b'-')
        && bytes[10] == b'_'
        && [13, 16].iter().all(|at| bytes[*at] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(at, byte)| matches!(at, 4 | 7 | 10 | 13 | 16) || byte.is_ascii_digit())
}

/// The manifest an archive carries. Identity is `format` + `world`; `backup` is for a reader.
#[must_use]
pub fn manifest(world: &str, backup: &str) -> String {
    format!(
        "{{\"format\":\"{MANIFEST_FORMAT}\",\"world\":{},\"backup\":{}}}",
        json_string(world),
        json_string(backup)
    )
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Whether a manifest names this port and this world.
#[must_use]
pub fn manifest_identifies(text: &str, world: &str) -> bool {
    json_field(text, "format").as_deref() == Some(MANIFEST_FORMAT)
        && json_field(text, "world").as_deref() == Some(world)
}

/// One string field of the port's own flat manifest (no nested objects).
fn json_field(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let at = text.find(&needle)? + needle.len();
    let rest = text[at..].trim_start().strip_prefix(':')?.trim_start();
    let mut chars = rest.strip_prefix('"')?.chars();
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let digits: String = chars.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&digits, 16).ok()?)?);
                }
                _ => return None,
            },
            ch => out.push(ch),
        }
    }
    None
}

/// The archives oldest first: the UTC stamp, then the **numeric** generation. Text order puts
/// `-10` before `-2` and the base name after every `-N`, which would keep the wrong archive.
fn chronological(archives: &[Archive]) -> Vec<&Archive> {
    let mut ordered: Vec<&Archive> = archives.iter().collect();
    ordered.sort_by(|left, right| {
        (left.stamp.as_str(), left.generation).cmp(&(right.stamp.as_str(), right.generation))
    });
    ordered
}

/// Archives the `keep` rule removes, oldest first. The caller passes only archives it has
/// identified as this port's, and `protected` — the archive the publish just wrote — is never
/// among them: reusing a pruned suffix or a clock that stepped back can leave the new archive
/// behind an older one, and deleting it would lose the backup this run just made.
#[must_use]
pub fn prune(archives: &[Archive], keep: usize, protected: &str) -> Vec<String> {
    let keep = keep.max(1);
    let ordered = chronological(archives);
    let mut remaining = ordered.len();
    let mut remove = Vec::new();
    for archive in ordered {
        if remaining <= keep {
            break;
        }
        if archive.name == protected {
            continue;
        }
        remove.push(archive.name.clone());
        remaining -= 1;
    }
    remove
}

/// The journal an operator's adoptions are recorded in, inside the backup folder. It is not a
/// `.zip`, so `/simplebackups list` and retention never mistake it for an archive.
pub const ADOPTION_FILE: &str = "simplebackups-adopted.txt";

/// One archive an operator adopted: the file name and the size it had when it was adopted.
///
/// The size is recorded so that replacing the file (a different archive under the same name)
/// drops the adoption: retention then stops touching it again. A name alone would let anything
/// that later lands on that path be deleted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adoption {
    pub name: String,
    pub bytes: u64,
}

/// Read the adoption journal: one `<bytes>\t<name>` line per adopted archive.
#[must_use]
pub fn parse_adoptions(text: &str) -> Vec<Adoption> {
    text.lines()
        .filter_map(|line| {
            let (bytes, name) = line.split_once('\t')?;
            let bytes = bytes.trim().parse().ok()?;
            let name = name.trim_end_matches('\r');
            if name.is_empty() || name.contains('\t') {
                return None;
            }
            Some(Adoption {
                name: name.to_owned(),
                bytes,
            })
        })
        .collect()
}

/// The journal as it is written back: stable order, one line per archive.
#[must_use]
pub fn format_adoptions(adoptions: &[Adoption]) -> String {
    adoptions
        .iter()
        .map(|adoption| format!("{}\t{}\n", adoption.bytes, adoption.name))
        .collect()
}

/// Whether this exact file (name and size) is one an operator adopted.
#[must_use]
pub fn is_adopted(adoptions: &[Adoption], name: &str, bytes: u64) -> bool {
    adoptions
        .iter()
        .any(|adoption| adoption.name == name && adoption.bytes == bytes)
}

fn normalise(relative: &str) -> Option<String> {
    let relative = relative.replace('\\', "/");
    let relative = relative.trim_start_matches("./");
    if relative.is_empty()
        || relative.starts_with('/')
        || relative.split('/').any(|part| part == "..")
    {
        return None;
    }
    Some(relative.to_owned())
}

/// A zip entry name: `<world>/<relative>`, or None for an unsafe path.
pub fn entry_name(world: &str, relative: &str) -> Option<String> {
    let relative = normalise(relative)?;
    Some(format!("{world}/{relative}"))
}

/// The single flat entry the captured game log is written under (upstream's `backup.log`).
pub const BACKUP_LOG_ENTRY: &str = "backup.log";

/// The game log's path under the game's home folder (upstream's `logs/latest.log`).
pub const GAME_LOG_RELATIVE: &str = "logs/latest.log";

/// The entry-name rule for the captured log: one flat name, never a path. The log
/// sits beside the world tree in the archive, not inside it, so a nested or
/// parent-traversing name is refused (upstream writes a single `backup.log`).
#[must_use]
pub fn backup_log_entry(name: &str) -> Option<&str> {
    (name == BACKUP_LOG_ENTRY).then_some(name)
}

/// Resolve the game log under `home` from its relative name. Only the exact
/// `logs/latest.log` is accepted: `normalise` refuses an absolute path and any
/// `..` component, and the equality check refuses a different file. The glue
/// additionally refuses a symlink and any path that canonicalises out of `home`.
#[must_use]
pub fn game_log_path(home: &Path, relative: &str) -> Option<PathBuf> {
    (normalise(relative)? == GAME_LOG_RELATIVE).then(|| home.join(GAME_LOG_RELATIVE))
}

/// Whether a world-relative path is left out of the zip.
pub fn should_skip(relative: &str, backups_prefix: Option<&str>, config: &Settings) -> bool {
    let Some(relative) = normalise(relative) else {
        return true;
    };
    let name = relative.rsplit('/').next().unwrap_or(&relative);
    if name == "session.lock" || name.ends_with(".part") || name.ends_with(".tmp") {
        return true;
    }
    // The port's own state files sit beside the world and are written between backups. They
    // would otherwise show up as "changed" in the next child, which is about world files only.
    if name.starts_with("simplebackups-") && name.ends_with(".txt") {
        return true;
    }
    if let Some(prefix) = backups_prefix.and_then(normalise) {
        // Normalise the configured prefix exactly as the visited path was
        // normalised: `./backups` names the same folder as `backups`, and comparing
        // the raw spelling would put old archives inside the new ones.
        if relative == prefix || relative.starts_with(&format!("{prefix}/")) {
            return true;
        }
    }
    if config.ignored_paths.iter().any(|prefix| {
        normalise(prefix)
            .is_some_and(|prefix| relative == prefix || relative.starts_with(&format!("{prefix}/")))
    }) {
        return true;
    }
    config
        .ignored_files
        .iter()
        .any(|file| normalise(file).is_some_and(|file| file == relative))
}

/// `12.3 MB` for a byte count.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Upstream's `maxDiskSize` default: `"25 GB"`, binary units (`CommonConfig.java:80-83`).
pub const DEFAULT_MAX_DISK_BYTES: u64 = 25 * 1024 * 1024 * 1024;

/// Upstream's `fullBackupTimer` default: 525960 minutes (~1 year), so a chain is
/// effectively permanent unless the operator lowers it (`CommonConfig.java:67-68`).
pub const DEFAULT_FULL_BACKUP_TIMER_MINUTES: u64 = 525_960;

/// The size units the config accepts, largest first, all binary (`StorageSize.java:32-37`).
const SIZE_UNITS: [(&str, u64); 5] = [
    ("TB", 1024 * 1024 * 1024 * 1024),
    ("GB", 1024 * 1024 * 1024),
    ("MB", 1024 * 1024),
    ("KB", 1024),
    ("B", 1),
];

/// Parse an upstream-style size: a whole number and an optional binary unit
/// (`B`, `KB`, `MB`, `GB`, `TB`), case-insensitively. A bare number is bytes.
#[must_use]
pub fn parse_size(input: &str) -> Option<u64> {
    let text = input.trim();
    let text = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(text)
        .trim();
    if text.is_empty() {
        return None;
    }
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    if number.is_empty() {
        return None;
    }
    let number: u64 = number.parse().ok()?;
    let unit = unit.trim().to_ascii_uppercase();
    let (_, multiplier) = SIZE_UNITS.iter().find(|(name, _)| {
        if unit.is_empty() {
            *name == "B"
        } else {
            *name == unit
        }
    })?;
    number.checked_mul(*multiplier)
}

/// A byte count in the config's own spelling, in the largest unit that divides it exactly.
#[must_use]
pub fn format_size_setting(bytes: u64) -> String {
    for (name, size) in SIZE_UNITS {
        if bytes >= size && bytes.is_multiple_of(size) {
            return format!("{} {name}", bytes / size);
        }
    }
    format!("{bytes} B")
}

/// What the disk cap does to a folder: the archives to delete (oldest first) and the size left.
#[derive(Debug, PartialEq, Eq)]
pub struct CapPlan {
    pub remove: Vec<String>,
    pub remaining_bytes: u64,
    pub over: bool,
}

/// Plan the archives the cap removes: the whole folder counts toward the cap, only archives the
/// caller identified as this port's are ever deleted, oldest first, never the last one left and
/// never `protected` (the archive this publish just wrote).
#[must_use]
pub fn cap_removals(
    entries: &[(String, u64)],
    archives: &[Archive],
    cap: u64,
    protected: &str,
) -> CapPlan {
    let mut remaining_bytes: u64 = entries.iter().map(|(_, size)| *size).sum();
    let ordered = chronological(archives);
    let mut remove = Vec::new();
    let mut remaining = ordered.len();
    let mut index = 0;
    while remaining_bytes > cap && remaining > 1 && index < ordered.len() {
        let archive = ordered[index];
        index += 1;
        if archive.name == protected {
            continue;
        }
        remove.push(archive.name.clone());
        remaining_bytes = remaining_bytes.saturating_sub(archive.bytes);
        remaining -= 1;
    }
    CapPlan {
        remove,
        remaining_bytes,
        over: remaining_bytes > cap,
    }
}

// --- Incremental chains (0.2.7) ------------------------------------------------
//
// Upstream's `BackupChain` / `BackupChainManager` / `ExperimentalConfig` (read at
// `bf04de3e`, branch `1.21.1`): an incremental chain is a **base full backup**
// plus zero or more **children** that carry only the files changed since the
// previous backup (INCREMENTAL) or the last full (DIFFERENTIAL). A compact
// `metadata.json` in the chain directory names `backupType`, `fullBackup`,
// `children` and `lastUpdated`; a child is `child-%04d.zip` (`index + 1`). This
// port keeps its flat `<world>_<UTC>.zip` archives and retention, so the same
// facts travel in each archive's own `simplebackups.json` instead.

/// Upstream's `ExperimentalConfig.ExperimentalBackupType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupType {
    /// Every backup carries the whole world (`FULL_BACKUPS`).
    Full,
    /// Changed since the previous backup (`INCREMENTAL` / `MODIFIED_SINCE_LAST`).
    Incremental,
    /// Changed since the last full (`DIFFERENTIAL` / `MODIFIED_SINCE_FULL`).
    Differential,
}

impl BackupType {
    /// The config spelling, also the value upstream's metadata stores.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Incremental => "incremental",
            Self::Differential => "differential",
        }
    }
}

/// Whether a backup is the chain's full base or a changed-only child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupKind {
    Full,
    Child,
}

/// The chain facts the port persists beside the world, mirroring upstream's
/// `BackupData.lastSaved` / `lastFullBackup` plus which archives form the chain.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChainState {
    pub base: Option<String>,
    pub children: Vec<String>,
    pub len: usize,
    /// When the most recent backup started: the threshold for the next child.
    pub last_backup_ms: u64,
    /// When the most recent full base started: the differential threshold.
    pub last_full_ms: u64,
}

/// The chain state file, beside the world like the pause and schedule files.
pub const CHAIN_FILE: &str = "simplebackups-chain.txt";

/// The manifest each archive carries. It keeps the `simplebackups/1` identity so retention is
/// untouched, and adds the chain facts upstream keeps in its per-chain `metadata.json`: the base
/// full backup, whether this is the full base or a child, the entries this archive actually holds,
/// and the chain's length so far. Children carry only the entries whose mtime passed the threshold.
#[must_use]
pub fn chain_manifest(
    world: &str,
    backup: &str,
    base: &str,
    kind: BackupKind,
    changed: &[String],
    len: usize,
) -> String {
    let entries: Vec<String> = changed.iter().map(|name| json_string(name)).collect();
    let kind = match kind {
        BackupKind::Full => "full",
        BackupKind::Child => "child",
    };
    format!(
        "{{\"format\":\"{MANIFEST_FORMAT}\",\"world\":{},\"backup\":{},\
         \"chain\":{},\"kind\":\"{kind}\",\"changed\":[{}],\"len\":{len}}}",
        json_string(world),
        json_string(backup),
        json_string(base),
        entries.join(",")
    )
}

/// Parse a backup type from the config or upstream's metadata. Upstream's enum names
/// (`FULL_BACKUPS`, `INCREMENTAL`/`MODIFIED_SINCE_LAST`, `DIFFERENTIAL`/`MODIFIED_SINCE_FULL`)
/// are accepted as well as the port's shorter spelling.
#[must_use]
pub fn parse_backup_type(value: &str) -> Option<BackupType> {
    let value = value.trim();
    let value = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value);
    match value.to_ascii_uppercase().as_str() {
        "FULL" | "FULL_BACKUPS" => Some(BackupType::Full),
        "INCREMENTAL" | "MODIFIED_SINCE_LAST" => Some(BackupType::Incremental),
        "DIFFERENTIAL" | "MODIFIED_SINCE_FULL" => Some(BackupType::Differential),
        _ => None,
    }
}

/// Whether the periodic-full timer has elapsed (upstream `fullBackupTimer`, minutes).
#[must_use]
pub fn full_timer_due(now_ms: u64, last_full_ms: u64, timer_minutes: u64) -> bool {
    timer_minutes > 0 && now_ms.saturating_sub(last_full_ms) >= timer_minutes.saturating_mul(60_000)
}

/// Which kind of backup starts next (upstream `getChainBackupFilePath`): a full backup when
/// full-mode, when no chain exists yet, or when the periodic-full timer is due; otherwise a child.
#[must_use]
pub fn choose_kind(backup_type: BackupType, has_base: bool, timer_due: bool) -> BackupKind {
    if backup_type == BackupType::Full || !has_base || timer_due {
        BackupKind::Full
    } else {
        BackupKind::Child
    }
}

/// The mtime threshold a child filters by: `lastSaved` for incremental, `lastFullBackup` for
/// differential (upstream `BackupThread.java:76`).
#[must_use]
pub fn changed_threshold(backup_type: BackupType, state: &ChainState) -> u64 {
    match backup_type {
        BackupType::Differential => state.last_full_ms,
        BackupType::Full | BackupType::Incremental => state.last_backup_ms,
    }
}

/// Whether one file belongs in this archive. Upstream's `BackupThread.java:395` tests
/// `lastModified - lastSaved > 0` (strictly newer), and a full base includes every file.
#[must_use]
pub fn entry_changed(modified_ms: u64, threshold_ms: u64, full: bool) -> bool {
    full || modified_ms > threshold_ms
}

/// The zips a merge reads, base first (upstream `MergeCommand.java:70-81`): a full chain merges
/// just its base, an incremental chain the base and every child, a differential chain the base
/// and only its newest child.
#[must_use]
pub fn merge_order(backup_type: BackupType, base: &str, children: &[String]) -> Vec<String> {
    match backup_type {
        BackupType::Full => vec![base.to_owned()],
        BackupType::Incremental => {
            let mut order = vec![base.to_owned()];
            order.extend(children.iter().cloned());
            order
        }
        BackupType::Differential => {
            let mut order = vec![base.to_owned()];
            if let Some(last) = children.last() {
                order.push(last.clone());
            }
            order
        }
    }
}

/// The chain state as exact one-line keys. `-` is "no base yet" (the world has never been backed
/// up), so an absent base does not have to be special-cased in the file's shape.
#[must_use]
pub fn format_chain(state: &ChainState) -> String {
    let mut out = format!(
        "base = {}\nlen = {}\nlast = {}\nfull = {}\n",
        state.base.as_deref().unwrap_or("-"),
        state.len,
        state.last_backup_ms,
        state.last_full_ms
    );
    for child in &state.children {
        out.push_str("child = ");
        out.push_str(child);
        out.push('\n');
    }
    out
}

/// Read the chain state. Only a file this port wrote parses: all four keys must be present and
/// well-formed; anything else leaves the chain fresh, exactly like a missing file.
#[must_use]
pub fn parse_chain(text: &str) -> Option<ChainState> {
    let mut base = None;
    let mut children = Vec::new();
    let mut len = None;
    let mut last = None;
    let mut full = None;
    for line in text.lines() {
        let (key, value) = line.split_once(" = ")?;
        match key {
            "base" => {
                base = if value == "-" {
                    None
                } else if value.is_empty() || value.contains('\t') {
                    return None;
                } else {
                    Some(value.to_owned())
                }
            }
            "child" => {
                if value.is_empty() || value.contains('\t') {
                    return None;
                }
                children.push(value.to_owned());
            }
            "len" => len = Some(value.parse().ok()?),
            "last" => last = Some(value.parse().ok()?),
            "full" => full = Some(value.parse().ok()?),
            _ => return None,
        }
    }
    Some(ChainState {
        base,
        children,
        len: len?,
        last_backup_ms: last?,
        last_full_ms: full?,
    })
}

/// The base named by a chain manifest.
#[must_use]
pub fn manifest_chain(text: &str) -> Option<String> {
    json_field(text, "chain")
}

/// The kind named by a chain manifest.
#[must_use]
pub fn manifest_kind(text: &str) -> Option<BackupKind> {
    match json_field(text, "kind").as_deref() {
        Some("full") => Some(BackupKind::Full),
        Some("child") => Some(BackupKind::Child),
        _ => None,
    }
}

/// The changed entries named by a chain manifest (a flat array of JSON strings).
#[must_use]
pub fn manifest_changed(text: &str) -> Vec<String> {
    let needle = "\"changed\"";
    let Some(at) = text.find(needle).map(|at| at + needle.len()) else {
        return Vec::new();
    };
    let Some(rest) = text[at..]
        .trim_start()
        .strip_prefix(':')
        .map(str::trim_start)
    else {
        return Vec::new();
    };
    let Some(inner) = rest
        .strip_prefix('[')
        .and_then(|r| r.split_once(']').map(|(v, _)| v))
    else {
        return Vec::new();
    };
    inner
        .split(',')
        .filter_map(|item| unquote_json(item.trim()))
        .collect()
}

/// The chain length named by a chain manifest.
#[must_use]
pub fn manifest_len(text: &str) -> Option<usize> {
    let needle = "\"len\"";
    let at = text.find(needle)? + needle.len();
    let rest = text[at..].trim_start().strip_prefix(':')?.trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Unescape one JSON string literal (`json_string`'s inverse).
fn unquote_json(value: &str) -> Option<String> {
    let mut chars = value.strip_prefix('"')?.chars();
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let digits: String = chars.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&digits, 16).ok()?)?);
                }
                _ => return None,
            },
            ch => out.push(ch),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_upstream() {
        let config = Settings::default();
        assert!(config.enabled);
        assert_eq!(config.interval_minutes, 120);
        assert_eq!(config.keep, 10);
        assert_eq!(config.compression_level, -1);
        assert!(config.save_before_backup);
        assert!(config.broadcast);
        assert_eq!(config.backups_dir(), "backups");
    }

    #[test]
    fn parse_overrides() {
        let (config, warnings) = parse(
            r#"
            enabled = false
            interval_minutes = 5
            interval_ticks = 40
            keep = 2
            compression_level = 6
            save_before_backup = false
            backups_dir = "snapshots"
            ignored_paths = ["region", "data/raids"]
            ignored_files = ["level.dat_old"]
            "#,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(!config.enabled);
        assert_eq!(config.interval_minutes, 5);
        assert_eq!(config.interval_ticks, 40);
        assert_eq!(config.keep, 2);
        assert_eq!(config.compression_level, 6);
        assert!(!config.save_before_backup);
        assert_eq!(config.backups_dir(), "snapshots");
        assert_eq!(config.ignored_paths, ["region", "data/raids"]);
        assert_eq!(config.ignored_files, ["level.dat_old"]);
    }

    #[test]
    fn parse_bad_values_keep_defaults_and_warn() {
        let (config, warnings) = parse("keep = x\ncompression_level = 99\nnope = 1\n");
        assert_eq!(config.keep, 10);
        assert_eq!(config.compression_level, 9);
        assert_eq!(warnings.len(), 3);
    }

    #[test]
    fn keep_is_clamped_to_one() {
        let (config, warnings) = parse("keep = 0\n");
        assert_eq!(config.keep, 1);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn due_by_ticks_boundaries() {
        assert!(!due_by_ticks(0, 19, 20));
        assert!(due_by_ticks(0, 20, 20));
        assert!(!due_by_ticks(20, 39, 20));
        assert!(!due_by_ticks(0, 39, 0));
    }

    #[test]
    fn due_by_millis_boundaries() {
        assert!(!due_by_millis(0, 7_199_999, 120));
        assert!(due_by_millis(0, 7_200_000, 120));
        assert!(!due_by_millis(0, 7_200_000, 0));
    }

    #[test]
    fn a_paused_schedule_never_starts_a_backup() {
        assert!(
            schedule_allowed(true, false),
            "an enabled, unpaused schedule runs"
        );
        assert!(
            !schedule_allowed(true, true),
            "pausing suppresses only the schedule"
        );
        assert!(
            !schedule_allowed(false, false),
            "enabled=false still gates it"
        );
        assert!(
            !schedule_allowed(false, true),
            "both gates together stay shut"
        );
    }

    #[test]
    fn the_pause_flag_round_trips_through_its_file() {
        assert_eq!(parse_paused(&format_paused(true)), Some(true));
        assert_eq!(parse_paused(&format_paused(false)), Some(false));
        assert_eq!(parse_paused(""), None, "a missing flag file means unpaused");
        assert_eq!(
            parse_paused("paused = yes\n"),
            None,
            "a malformed flag is refused"
        );
    }

    #[test]
    fn the_persisted_tick_counter_round_trips_through_its_file() {
        let text = format_schedule_ticks(17);
        assert_eq!(text, "ticks = 17\n", "the file is one exact line");
        assert_eq!(parse_schedule_ticks(&text), Some(17));
        assert_eq!(
            parse_schedule_ticks(""),
            None,
            "no file means a fresh schedule"
        );
        assert_eq!(
            parse_schedule_ticks("ticks = nope\n"),
            None,
            "malformed is refused"
        );
        assert_eq!(
            parse_schedule_ticks("paused = true\n"),
            None,
            "another key is refused"
        );
    }

    #[test]
    fn a_restored_counter_resumes_the_interval() {
        // A saved progress of 12 at a 40-tick interval: 20 more ticks are not due, 28 are.
        assert_eq!(schedule_progress(52, 40), 12);
        assert!(!due_by_ticks(0, 12 + 20, 40));
        assert!(due_by_ticks(0, 12 + 28, 40));
    }

    #[test]
    fn names_are_utc_and_sortable() {
        assert_eq!(backup_file_stem("W", 0), "W_1970-01-01_00-00-00");
        assert_eq!(
            backup_file_stem("W", 1_793_709_296),
            "W_2026-11-03_12-34-56"
        );
        assert_eq!(
            backup_file_stem("W", 1_709_164_800),
            "W_2024-02-29_00-00-00"
        );
    }

    #[test]
    fn name_collision_gets_a_suffix() {
        let base = "W_2026-11-03_12-34-56.zip".to_owned();
        assert_eq!(backup_file_name("W", 1_793_709_296, &[]), base);
        assert_eq!(
            backup_file_name("W", 1_793_709_296, std::slice::from_ref(&base)),
            "W_2026-11-03_12-34-56-1.zip"
        );
        assert_eq!(
            backup_file_name(
                "W",
                1_793_709_296,
                &[base, "W_2026-11-03_12-34-56-1.zip".to_owned()]
            ),
            "W_2026-11-03_12-34-56-2.zip"
        );
    }

    #[test]
    fn a_pruned_suffix_is_never_reused_for_a_new_backup() {
        // Retention pruned generations 0..=4 of this second, so only `-5` is left and the
        // base name is free. Reusing it would make the new backup the *oldest* of its
        // second, and the pass that runs right after would delete it as the oldest.
        let stem = "W_2026-11-03_12-34-56";
        let taken = vec![format!("{stem}-5.zip")];
        assert_eq!(
            backup_file_name("W", 1_793_709_296, &taken),
            format!("{stem}-6.zip"),
            "the next generation is above the highest suffix, not the lowest free one"
        );
        // A gap below the highest suffix is not free either: `-1` would sort before `-3`.
        let taken = vec![format!("{stem}.zip"), format!("{stem}-3.zip")];
        assert_eq!(
            backup_file_name("W", 1_793_709_296, &taken),
            format!("{stem}-4.zip")
        );
        // Only a folder with no archive of this second starts at the base name.
        assert_eq!(
            backup_file_name("W", 1_793_709_296, &[]),
            format!("{stem}.zip")
        );
    }

    #[test]
    fn a_name_with_a_multibyte_character_at_the_stamp_boundary_is_refused() {
        // 18 ASCII bytes, then a two-byte character: byte 19 is inside it, so slicing the
        // stamp by bytes panics — on the server tick, which is a crash, not a warning.
        assert!(
            parse_archive("W_2026-01-01_00-00-0é.zip", "W", 1).is_none(),
            "a malformed name is refused, never sliced at a non-character boundary"
        );
        assert!(parse_archive("W_é026-01-01_00-00-00.zip", "W", 1).is_none());
        assert!(parse_archive("W_2026-01-01_00-00-00é.zip", "W", 1).is_none());
        // The same name in ASCII still parses, so the check is about the boundary only.
        assert!(parse_archive("W_2026-01-01_00-00-00.zip", "W", 1).is_some());
    }

    /// The archives named, as the glue builds them after it has identified each one.
    fn archives(names: &[&str]) -> Vec<Archive> {
        names
            .iter()
            .map(|name| parse_archive(name, "W", 1).expect("a backup name"))
            .collect()
    }

    fn archive(name: &str, bytes: u64) -> Archive {
        parse_archive(name, "W", bytes).expect("a backup name")
    }

    #[test]
    fn prune_keeps_the_newest() {
        let names = [
            "W_2026-01-01_00-00-01.zip",
            "W_2026-01-01_00-00-02.zip",
            "W_2026-01-01_00-00-03.zip",
        ];
        let candidates = archives(&names);
        assert_eq!(prune(&candidates, 2, ""), [names[0]]);
        assert_eq!(prune(&candidates, 1, ""), [names[0], names[1]]);
        assert!(prune(&candidates, 3, "").is_empty());
    }

    #[test]
    fn a_name_that_is_not_a_backup_is_never_an_archive() {
        // Retention is driven by parsed names, not by a prefix test: a hand-made ZIP that
        // merely shares the world prefix has no archive identity to delete.
        assert!(parse_archive("World_notes.zip", "World", 1).is_none());
        assert!(parse_archive("World_2026-01-01_00-00-00.zip.part", "World", 1).is_none());
        assert!(parse_archive("Other_2026-01-01_00-00-00.zip", "World", 1).is_none());
        assert!(parse_archive("World_2026-01-01_00-00-00-1.zip", "World", 1).is_some());
        assert!(parse_archive("World_2026-01-01_00-00-00-x.zip", "World", 1).is_none());
        assert!(parse_archive("World_2026-01-01_00-00-00-.zip", "World", 1).is_none());
    }

    #[test]
    fn entry_names_are_world_prefixed_and_safe() {
        assert_eq!(
            entry_name("World", "region/r.0.0.mca").as_deref(),
            Some("World/region/r.0.0.mca")
        );
        assert_eq!(
            entry_name("World", "region\\r.0.0.mca").as_deref(),
            Some("World/region/r.0.0.mca")
        );
        assert_eq!(entry_name("World", "../x"), None);
        assert_eq!(entry_name("World", "/etc/passwd"), None);
        assert_eq!(entry_name("World", ""), None);
    }

    #[test]
    fn skip_rules_cover_locks_temp_and_the_backups_folder() {
        let config = Settings::default();
        assert!(should_skip("session.lock", Some("backups"), &config));
        assert!(should_skip(
            "region/r.0.0.mca.part",
            Some("backups"),
            &config
        ));
        assert!(should_skip(
            "region/r.0.0.mca.tmp",
            Some("backups"),
            &config
        ));
        assert!(should_skip("backups/W_1.zip", Some("backups"), &config));
        assert!(should_skip("backups", Some("backups"), &config));
        assert!(!should_skip("region/r.0.0.mca", Some("backups"), &config));
        assert!(!should_skip("level.dat", Some("backups"), &config));
    }

    #[test]
    fn a_backup_folder_may_not_leave_the_world() {
        assert_eq!(safe_backups_dir("backups").as_deref(), Some("backups"));
        assert_eq!(safe_backups_dir("./backups").as_deref(), Some("backups"));
        assert_eq!(safe_backups_dir("backups/").as_deref(), Some("backups"));
        assert_eq!(
            safe_backups_dir("backups/worlds").as_deref(),
            Some("backups/worlds")
        );
        assert_eq!(safe_backups_dir("../outside"), None);
        assert_eq!(safe_backups_dir("backups/../../etc"), None);
        assert_eq!(safe_backups_dir("/etc/simplebackups"), None);
        assert_eq!(
            safe_backups_dir("."),
            None,
            "the world itself is not a backup folder"
        );
        assert_eq!(safe_backups_dir("   "), None);
    }

    #[test]
    fn an_unsafe_backups_dir_is_refused_and_the_default_kept() {
        let (config, warnings) = parse("backups_dir = \"../outside\"\n");
        assert_eq!(config.backups_dir(), "backups");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        let (config, warnings) = parse("backups_dir = \"/srv/backups\"\n");
        assert_eq!(config.backups_dir(), "backups");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[test]
    fn a_dot_spelled_output_folder_is_still_excluded_from_the_zip() {
        let config = Settings::default();
        // The walk normalises the visited path; the configured prefix must be
        // normalised the same way or the archives are copied into the next archive.
        assert!(should_skip("backups/W_1.zip", Some("./backups"), &config));
        assert!(!should_skip("region/r.0.0.mca", Some("./backups"), &config));
    }

    #[test]
    fn skip_rules_honour_the_config() {
        let (config, _) =
            parse("ignored_paths = [\"data/raids\"]\nignored_files = [\"level.dat_old\"]\n");
        assert!(should_skip("data/raids/raids.dat", None, &config));
        assert!(!should_skip("data/raids_other", None, &config));
        assert!(should_skip("level.dat_old", None, &config));
        assert!(!should_skip("level.dat", None, &config));
    }

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(12_900_000), "12.3 MB");
    }

    #[test]
    fn a_copy_overlapped_by_a_save_is_stale() {
        assert!(!copy_is_stale(7, 7));
        assert!(copy_is_stale(7, 8));
    }

    #[test]
    fn the_disk_cap_defaults_to_upstreams_25_gb() {
        let config = Settings::default();
        assert_eq!(
            config.max_disk_bytes, 26_843_545_600,
            "upstream's maxDiskSize default is 25 GB in binary units"
        );
        let written = to_toml(&config);
        assert!(
            written.contains("max_disk_size = \"25 GB\""),
            "the default file must carry the cap: {written}"
        );
    }

    #[test]
    fn sizes_are_parsed_with_binary_units() {
        assert_eq!(parse_size("25 GB"), Some(26_843_545_600));
        assert_eq!(parse_size("512 B"), Some(512));
        assert_eq!(parse_size("512"), Some(512), "a bare number is bytes");
        assert_eq!(parse_size("1 KB"), Some(1_024));
        assert_eq!(parse_size("2 MB"), Some(2 * 1024 * 1024));
        assert_eq!(parse_size("1 TB"), Some(1024 * 1024 * 1024 * 1024));
        assert_eq!(
            parse_size(" 25 gb "),
            Some(26_843_545_600),
            "spacing and case do not matter"
        );
        assert_eq!(
            parse_size("1.5 GB"),
            None,
            "upstream sizes are whole numbers"
        );
        assert_eq!(parse_size("25 GB extra"), None);
        assert_eq!(parse_size("GB"), None);
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("-1 KB"), None);
        assert_eq!(
            parse_size("99999999999 TB"),
            None,
            "a size that cannot be a u64 is refused, not wrapped"
        );
    }

    #[test]
    fn sizes_render_in_the_largest_exact_unit() {
        assert_eq!(format_size_setting(26_843_545_600), "25 GB");
        assert_eq!(format_size_setting(1024), "1 KB");
        assert_eq!(format_size_setting(512), "512 B");
        assert_eq!(
            format_size_setting(1_536),
            "1536 B",
            "only a unit that divides the count exactly is used"
        );
    }

    #[test]
    fn an_unparsable_disk_cap_keeps_the_default_and_warns() {
        let (config, warnings) = parse("max_disk_size = \"a lot\"\n");
        assert_eq!(config.max_disk_bytes, DEFAULT_MAX_DISK_BYTES);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        let (config, warnings) = parse("max_disk_size = \"1 KB\"\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(config.max_disk_bytes, 1_024);
    }

    #[test]
    fn the_cap_removes_the_oldest_archives_first_but_never_the_last() {
        let entries = vec![
            ("W_2026-01-01_00-00-01.zip".to_owned(), 600),
            ("W_2026-01-01_00-00-02.zip".to_owned(), 600),
            ("W_2026-01-01_00-00-03.zip".to_owned(), 600),
        ];
        let candidates = vec![
            archive("W_2026-01-01_00-00-01.zip", 600),
            archive("W_2026-01-01_00-00-02.zip", 600),
            archive("W_2026-01-01_00-00-03.zip", 600),
        ];
        let plan = cap_removals(&entries, &candidates, 500, "");
        assert_eq!(
            plan.remove,
            ["W_2026-01-01_00-00-01.zip", "W_2026-01-01_00-00-02.zip"]
        );
        assert_eq!(plan.remaining_bytes, 600, "the newest archive stays");
        assert!(plan.over, "one archive is still bigger than the cap");
        let plan = cap_removals(&entries, &candidates, 2_000, "");
        assert!(plan.remove.is_empty());
        assert_eq!(plan.remaining_bytes, 1_800);
        assert!(!plan.over);
    }

    #[test]
    fn the_cap_counts_the_folder_but_only_deletes_identified_archives() {
        // The size counts every file (so the cap cannot be hidden behind a foreign file), but
        // only an archive the caller identified is a deletion candidate.
        let entries = vec![
            ("W_2026-01-01_00-00-01.zip".to_owned(), 400),
            ("W_2026-01-01_00-00-02.zip".to_owned(), 400),
            ("Other_2026-01-01_00-00-01.zip".to_owned(), 400),
            ("someone-elses-map.zip".to_owned(), 5_000),
            ("notes.txt".to_owned(), 100),
        ];
        let candidates = vec![
            archive("W_2026-01-01_00-00-01.zip", 400),
            archive("W_2026-01-01_00-00-02.zip", 400),
        ];
        let plan = cap_removals(&entries, &candidates, 1_000, "");
        assert_eq!(
            plan.remove,
            ["W_2026-01-01_00-00-01.zip"],
            "nothing unidentified is deleted"
        );
        assert_eq!(plan.remaining_bytes, 5_900);
        assert!(plan.over);
    }

    #[test]
    fn the_default_file_round_trips() {
        let config = Settings::default();
        let (parsed, warnings) = parse(&to_toml(&config));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(parsed, config);
    }

    /// Eleven archives written inside one second: the base name is generation 0, then
    /// `-1` … `-10`. As text, `-10` sorts before `-2` and `-1` sorts before the base.
    fn same_second_names() -> Vec<String> {
        (0..=10)
            .map(|generation| {
                if generation == 0 {
                    "W_2026-11-03_12-34-56.zip".to_owned()
                } else {
                    format!("W_2026-11-03_12-34-56-{generation}.zip")
                }
            })
            .collect()
    }

    #[test]
    fn same_second_backups_sort_by_generation_not_as_text() {
        let names = same_second_names();
        let candidates: Vec<Archive> = names
            .iter()
            .map(|name| parse_archive(name, "W", 1).expect("a backup name"))
            .collect();
        let removed = prune(&candidates, 2, "");
        assert_eq!(
            removed,
            names[..9].to_vec(),
            "the oldest nine generations go, not the ones that sort first as text"
        );
        assert!(
            !removed.contains(&names[10]),
            "generation 10 is the newest, not the first to go"
        );
        assert!(
            !removed.contains(&names[9]),
            "generation 9 is second newest"
        );
    }

    #[test]
    fn the_cap_also_orders_same_second_generations_numerically() {
        let names = same_second_names();
        let entries: Vec<(String, u64)> = names.iter().cloned().map(|name| (name, 1_000)).collect();
        let candidates: Vec<Archive> = names
            .iter()
            .map(|name| parse_archive(name, "W", 1_000).expect("a backup name"))
            .collect();
        let plan = cap_removals(&entries, &candidates, 2_000, "");
        assert_eq!(
            plan.remove,
            names[..9].to_vec(),
            "the cap must delete generations 0-8, never the numerically newest"
        );
        assert_eq!(plan.remaining_bytes, 2_000);
        assert!(!plan.over);
    }

    #[test]
    fn neither_pass_removes_the_archive_just_published() {
        // The archive just written can sort *oldest*: a reused pruned suffix, or a clock that
        // stepped back. Deleting it would lose the backup this run just made, so both passes
        // take it out of their candidates, and the cap may leave the folder over its limit
        // rather than delete it.
        let names = [
            "W_2026-11-03_12-34-56.zip",
            "W_2026-11-03_12-34-56-1.zip",
            "W_2026-11-03_12-34-56-2.zip",
        ];
        let candidates = archives(&names);
        let protected = names[0];
        assert_eq!(
            prune(&candidates, 1, protected),
            [names[1].to_owned(), names[2].to_owned()],
            "keep removes the others, never the archive just written"
        );
        let entries: Vec<(String, u64)> =
            names.iter().map(|name| ((*name).to_owned(), 100)).collect();
        let plan = cap_removals(&entries, &candidates, 50, protected);
        assert_eq!(plan.remove, [names[1].to_owned(), names[2].to_owned()]);
        assert!(
            plan.over,
            "the protected archive alone is over the cap and still stays"
        );
        // The same set without protection shows what would be lost: the oldest goes first.
        assert_eq!(
            prune(&candidates, 1, names[2]),
            [names[0].to_owned(), names[1].to_owned()]
        );
    }

    #[test]
    fn the_adoption_journal_round_trips_and_matches_the_size() {
        let adoptions = vec![
            Adoption {
                name: "W_2026-01-01_00-00-00.zip".to_owned(),
                bytes: 1_234,
            },
            Adoption {
                name: "W_2026-01-02_00-00-00.zip".to_owned(),
                bytes: 42,
            },
        ];
        assert_eq!(parse_adoptions(&format_adoptions(&adoptions)), adoptions);
        assert!(is_adopted(&adoptions, "W_2026-01-01_00-00-00.zip", 1_234));
        assert!(
            !is_adopted(&adoptions, "W_2026-01-01_00-00-00.zip", 1_235),
            "a file replaced since the adoption is not the adopted archive"
        );
        assert!(!is_adopted(&adoptions, "W_2026-01-03_00-00-00.zip", 1_234));
        assert!(parse_adoptions("garbage\n\n").is_empty());
    }

    #[test]
    fn the_manifest_identifies_this_port_and_world_only() {
        let text = manifest("World", "World_2026-11-03_12-34-56.zip");
        assert!(manifest_identifies(&text, "World"));
        assert!(!manifest_identifies(&text, "Other"));
        assert!(!manifest_identifies(
            "{\"format\":\"other/1\",\"world\":\"World\"}",
            "World"
        ));
        assert!(!manifest_identifies("not a manifest", "World"));
        assert!(
            !manifest_identifies("{\"format\":\"simplebackups/1\"}", "World"),
            "a manifest without a world is not an identity"
        );
        let quoted = manifest("World \"quoted\"", "x.zip");
        assert!(
            manifest_identifies(&quoted, "World \"quoted\""),
            "the writer escapes what the reader unescapes: {quoted}"
        );
    }

    // --- Incremental chains (0.2.7) -------------------------------------------

    #[test]
    fn a_changed_entry_is_one_modified_after_the_threshold() {
        assert!(
            entry_changed(2_000, 1_000, false),
            "a file written after the previous backup is changed"
        );
        assert!(
            !entry_changed(1_000, 1_000, false),
            "the boundary is strict: upstream tests lastModified - lastSaved > 0"
        );
        assert!(
            !entry_changed(500, 1_000, false),
            "an older file is not changed"
        );
        assert!(
            entry_changed(1, u64::MAX, true),
            "a full base carries every file, whatever its mtime"
        );
    }

    #[test]
    fn the_first_backup_of_a_chain_is_full_and_later_ones_are_children() {
        assert_eq!(
            choose_kind(BackupType::Incremental, false, false),
            BackupKind::Full,
            "no base yet: the first backup is the full base"
        );
        assert_eq!(
            choose_kind(BackupType::Incremental, true, false),
            BackupKind::Child
        );
        assert_eq!(
            choose_kind(BackupType::Incremental, true, true),
            BackupKind::Full,
            "the periodic full timer forces a new base"
        );
        assert_eq!(
            choose_kind(BackupType::Full, true, false),
            BackupKind::Full,
            "FULL_BACKUPS never builds a chain"
        );
    }

    #[test]
    fn differential_changes_since_the_full_but_incremental_since_the_last() {
        let state = ChainState {
            last_backup_ms: 5_000,
            last_full_ms: 1_000,
            ..ChainState::default()
        };
        assert_eq!(changed_threshold(BackupType::Incremental, &state), 5_000);
        assert_eq!(changed_threshold(BackupType::Differential, &state), 1_000);
        // A file written after the full but before the last backup: incremental (threshold
        // 5_000) already carried it, so the child skips it; differential (threshold 1_000)
        // must carry it again because the last full did not see it.
        assert!(!entry_changed(
            3_000,
            changed_threshold(BackupType::Incremental, &state),
            false
        ));
        assert!(entry_changed(
            3_000,
            changed_threshold(BackupType::Differential, &state),
            false
        ));
    }

    #[test]
    fn the_full_timer_is_upstreams_minutes_to_milliseconds() {
        assert!(!full_timer_due(1_000, 0, 10));
        assert!(!full_timer_due(599_999, 0, 10));
        assert!(full_timer_due(600_000, 0, 10));
        assert!(
            !full_timer_due(u64::MAX, u64::MAX, 10),
            "no overflow into due"
        );
    }

    #[test]
    fn backup_types_parse_from_both_the_config_and_upstream_metadata() {
        assert_eq!(parse_backup_type("full"), Some(BackupType::Full));
        assert_eq!(
            parse_backup_type("incremental"),
            Some(BackupType::Incremental)
        );
        assert_eq!(
            parse_backup_type("differential"),
            Some(BackupType::Differential)
        );
        assert_eq!(parse_backup_type("FULL_BACKUPS"), Some(BackupType::Full));
        assert_eq!(
            parse_backup_type("INCREMENTAL"),
            Some(BackupType::Incremental)
        );
        assert_eq!(
            parse_backup_type("MODIFIED_SINCE_FULL"),
            Some(BackupType::Differential)
        );
        assert_eq!(parse_backup_type("nonsense"), None);
    }

    #[test]
    fn the_chain_manifest_names_the_base_kind_changed_entries_and_length() {
        let changed = vec!["World/changed.txt".to_owned(), "World/level.dat".to_owned()];
        let text = chain_manifest(
            "World",
            "World_2026-01-01_00-00-01.zip",
            "World_2026-01-01_00-00-00.zip",
            BackupKind::Child,
            &changed,
            2,
        );
        assert_eq!(
            manifest_chain(&text).as_deref(),
            Some("World_2026-01-01_00-00-00.zip")
        );
        assert_eq!(manifest_kind(&text), Some(BackupKind::Child));
        assert_eq!(manifest_changed(&text), changed);
        assert_eq!(manifest_len(&text), Some(2));
        assert!(
            manifest_identifies(&text, "World"),
            "the chain manifest still identifies this port and world for retention"
        );
        let full = chain_manifest(
            "World",
            "World_2026-01-01_00-00-00.zip",
            "World_2026-01-01_00-00-00.zip",
            BackupKind::Full,
            &[],
            1,
        );
        assert_eq!(manifest_kind(&full), Some(BackupKind::Full));
        assert_eq!(manifest_len(&full), Some(1));
        assert!(manifest_changed(&full).is_empty());
    }

    #[test]
    fn the_chain_state_round_trips_through_its_file() {
        let state = ChainState {
            base: Some("World_2026-01-01_00-00-00.zip".to_owned()),
            children: vec!["World_2026-01-01_00-00-01.zip".to_owned()],
            len: 2,
            last_backup_ms: 12_345,
            last_full_ms: 1_000,
        };
        let text = format_chain(&state);
        assert_eq!(parse_chain(&text), Some(state));
        assert_eq!(parse_chain(""), None, "a world with no chain starts fresh");
        assert_eq!(
            parse_chain("garbage\n"),
            None,
            "a hand-edited file is refused"
        );
    }

    #[test]
    fn merge_reads_the_full_and_the_right_children() {
        let base = "full.zip".to_owned();
        let children = vec!["child-0001.zip".to_owned(), "child-0002.zip".to_owned()];
        assert_eq!(
            merge_order(BackupType::Full, &base, &children),
            std::slice::from_ref(&base)
        );
        assert_eq!(
            merge_order(BackupType::Incremental, &base, &children),
            [base.clone(), children[0].clone(), children[1].clone()]
        );
        assert_eq!(
            merge_order(BackupType::Differential, &base, &children),
            [base, children[1].clone()],
            "a differential merge needs only the full and the newest child"
        );
    }

    #[test]
    fn an_empty_server_never_schedules_by_default() {
        // Upstream: noPlayerBackups=false means no scheduled backup while nobody is online.
        assert!(!scheduled_backup_permitted(true, false, false, false, 1, 0));
        // With a player online the same config still schedules.
        assert!(scheduled_backup_permitted(true, false, true, false, 1, 0));
    }

    #[test]
    fn no_player_count_caps_backups_per_empty_period() {
        // 0 disables empty-server backups; 1 allows one until a player joins and resets it;
        // 2 allows two. The counter is per empty period, not per save or per shutdown.
        assert!(!scheduled_backup_permitted(true, false, false, true, 0, 0));
        assert!(scheduled_backup_permitted(true, false, false, true, 1, 0));
        assert!(!scheduled_backup_permitted(true, false, false, true, 1, 1));
        assert!(scheduled_backup_permitted(true, false, false, true, 2, 1));
        assert!(!scheduled_backup_permitted(true, false, false, true, 2, 2));
        // The presence rule wraps the existing shouldRunBackup gate.
        assert!(!scheduled_backup_permitted(false, false, false, true, 1, 0));
        assert!(!scheduled_backup_permitted(true, true, false, true, 1, 0));
        assert!(scheduled_backup_permitted(true, false, true, true, 1, 9));
    }

    #[test]
    fn the_empty_backup_counter_round_trips_through_its_file() {
        assert_eq!(format_no_player_count(3), "backups = 3\n");
        assert_eq!(parse_no_player_count("backups = 3\n"), Some(3));
        assert_eq!(
            parse_no_player_count(""),
            None,
            "a world that never backed up empty"
        );
        assert_eq!(
            parse_no_player_count("backups = nope\n"),
            None,
            "a hand-edited file"
        );
        assert_eq!(parse_no_player_count("backups = 3 extra\n"), None);
    }

    #[test]
    fn no_player_keys_round_trip_through_the_config() {
        let (config, warnings) = parse("no_player_backups = true\nno_player_backup_count = 2\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(config.no_player_backups);
        assert_eq!(config.no_player_backup_count, 2);
        let defaults = Settings::default();
        assert!(!defaults.no_player_backups, "upstream's default is false");
        assert_eq!(defaults.no_player_backup_count, 1);
        let (round_trip, warnings) = parse(&to_toml(&config));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(round_trip, config);
        let (_, warnings) = parse("no_player_backup_count = -1\nno_player_backups = yes\n");
        assert_eq!(warnings.len(), 2, "bad values keep the defaults and warn");
    }

    #[test]
    fn the_game_log_is_stored_under_one_flat_entry_name() {
        assert_eq!(backup_log_entry("backup.log"), Some("backup.log"));
        assert_eq!(
            backup_log_entry("World/backup.log"),
            None,
            "the log is a flat entry beside the world tree"
        );
        assert_eq!(
            backup_log_entry("../backup.log"),
            None,
            "a path-shaped entry name is refused"
        );
    }

    #[test]
    fn the_log_capture_key_defaults_on_and_round_trips() {
        assert!(
            Settings::default().capture_latest_log,
            "upstream captureLatestLog defaults to true"
        );
        let (config, warnings) = parse("capture_latest_log = false\n");
        assert!(warnings.is_empty(), "the key must be known: {warnings:?}");
        assert!(
            !config.capture_latest_log,
            "the config must be able to disable capture"
        );
        let text = to_toml(&config);
        assert!(
            text.contains("capture_latest_log = false"),
            "the key must be written back: {text}"
        );
        let (again, warnings) = parse(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again, config);
    }

    #[test]
    fn a_game_log_path_may_not_leave_the_game_folder() {
        let home = Path::new("/game");
        assert_eq!(
            game_log_path(home, GAME_LOG_RELATIVE),
            Some(home.join("logs/latest.log"))
        );
        assert_eq!(
            game_log_path(home, "../latest.log"),
            None,
            "parent traversal must be refused"
        );
        assert_eq!(
            game_log_path(home, "/etc/passwd"),
            None,
            "an absolute path must be refused"
        );
        assert_eq!(
            game_log_path(home, "logs/debug.log"),
            None,
            "only latest.log is captured"
        );
    }

    #[test]
    fn an_explicit_empty_timer_replaces_the_main_interval() {
        // Upstream `getTimer(false)`: noPlayerBackupTimer is the empty period's interval.
        assert_eq!(empty_period_interval(false, 120, 7), 7);
    }

    #[test]
    fn a_zero_empty_timer_falls_back_to_the_main_interval() {
        // Upstream `CommonConfig.java:153-155`: 0 means the same as `timer`.
        assert_eq!(empty_period_interval(false, 120, 0), 120);
        // With a player online the main timer always wins, whatever the empty value.
        assert_eq!(empty_period_interval(true, 120, 7), 120);
        assert_eq!(empty_period_interval(true, 120, 0), 120);
    }

    #[test]
    fn the_no_player_timer_key_defaults_to_zero_and_round_trips() {
        assert_eq!(
            Settings::default().no_player_backup_timer_minutes,
            0,
            "upstream's default is 0"
        );
        let (config, warnings) = parse("no_player_backup_timer = 7\n");
        assert!(warnings.is_empty(), "the key must be known: {warnings:?}");
        assert_eq!(config.no_player_backup_timer_minutes, 7);
        let text = to_toml(&config);
        assert!(
            text.contains("no_player_backup_timer = 7"),
            "the key must be written back: {text}"
        );
        let (again, warnings) = parse(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again, config);
        let (_, warnings) = parse("no_player_backup_timer = -1\n");
        assert_eq!(warnings.len(), 1, "bad values keep the default and warn");
    }
}
