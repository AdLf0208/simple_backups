// Simple Backups for Horizon — the backup policy (pure).
//
// Ported from Simple Backups 1.21-4.0.30 (ChaoticTrials/SimpleBackups,
// Apache-2.0) for Minecraft Java 1.21.1 NeoForge. Upstream source read for
// behaviour only; this is a clean-room Rust rewrite.
//
// Slice 1: config, due time (ticks or wall clock), UTC zip names, retention,
// entry names and skip rules. The glue does the save, the zip, and the
// commands.

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

/// The next free `<stem>.zip`, with `-1`, `-2`… on a collision.
pub fn backup_file_name(world: &str, unix_secs: i64, taken: &[String]) -> String {
    let stem = backup_file_stem(world, unix_secs);
    let mut name = format!("{stem}.zip");
    let mut suffix = 0;
    while taken.iter().any(|candidate| candidate == &name) {
        suffix += 1;
        name = format!("{stem}-{suffix}.zip");
    }
    name
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

/// Backup names the retention policy may delete, oldest first.
pub fn prune(names: &[String], world: &str, keep: usize) -> Vec<String> {
    let keep = keep.max(1);
    let prefix = format!("{world}_");
    let mut candidates: Vec<&String> = names
        .iter()
        .filter(|name| {
            name.starts_with(&prefix) && name.ends_with(".zip") && !name.ends_with(".part")
        })
        .collect();
    candidates.sort();
    let remove = candidates.len().saturating_sub(keep);
    candidates.into_iter().take(remove).cloned().collect()
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

/// Whether a world-relative path is left out of the zip.
pub fn should_skip(relative: &str, backups_prefix: Option<&str>, config: &Settings) -> bool {
    let Some(relative) = normalise(relative) else {
        return true;
    };
    let name = relative.rsplit('/').next().unwrap_or(&relative);
    if name == "session.lock" || name.ends_with(".part") || name.ends_with(".tmp") {
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

/// Plan the archives the cap removes: the whole folder counts toward the cap, only this world's
/// `<world>_*.zip` files are ever deleted, oldest first, and never the last one standing.
#[must_use]
pub fn cap_removals(entries: &[(String, u64)], world: &str, cap: u64) -> CapPlan {
    let mut remaining_bytes: u64 = entries.iter().map(|(_, size)| *size).sum();
    let prefix = format!("{world}_");
    let mut candidates: Vec<(&str, u64)> = entries
        .iter()
        .filter(|(name, _)| name.starts_with(&prefix) && name.ends_with(".zip"))
        .map(|(name, size)| (name.as_str(), *size))
        .collect();
    // The names are UTC timestamps, so lexical order is chronological order.
    candidates.sort_by(|a, b| a.0.cmp(b.0));
    let mut remove = Vec::new();
    let mut index = 0;
    while remaining_bytes > cap && candidates.len().saturating_sub(index) > 1 {
        let (name, size) = candidates[index];
        remove.push(name.to_owned());
        remaining_bytes -= size;
        index += 1;
    }
    CapPlan {
        remove,
        remaining_bytes,
        over: remaining_bytes > cap,
    }
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
    fn prune_keeps_the_newest() {
        let names = vec![
            "W_1.zip".to_owned(),
            "W_2.zip".to_owned(),
            "W_3.zip".to_owned(),
        ];
        assert_eq!(prune(&names, "W", 2), ["W_1.zip"]);
        assert_eq!(prune(&names, "W", 1), ["W_1.zip", "W_2.zip"]);
        assert!(prune(&names, "W", 3).is_empty());
    }

    #[test]
    fn prune_ignores_foreign_files() {
        let names = vec![
            "other.zip".to_owned(),
            "W_1.zip.part".to_owned(),
            "notes.txt".to_owned(),
            "W_1.zip".to_owned(),
            "W_2.zip".to_owned(),
        ];
        assert_eq!(prune(&names, "W", 1), ["W_1.zip"]);
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
            ("W_1.zip".to_owned(), 600),
            ("W_2.zip".to_owned(), 600),
            ("W_3.zip".to_owned(), 600),
        ];
        let plan = cap_removals(&entries, "W", 500);
        assert_eq!(plan.remove, ["W_1.zip", "W_2.zip"]);
        assert_eq!(plan.remaining_bytes, 600, "the newest archive stays");
        assert!(plan.over, "one archive is still bigger than the cap");
        let plan = cap_removals(&entries, "W", 2_000);
        assert!(plan.remove.is_empty());
        assert_eq!(plan.remaining_bytes, 1_800);
        assert!(!plan.over);
    }

    #[test]
    fn the_cap_counts_the_folder_but_only_deletes_our_archives() {
        let entries = vec![
            ("W_1.zip".to_owned(), 400),
            ("W_2.zip".to_owned(), 400),
            ("Other_1.zip".to_owned(), 400),
            ("someone-elses-map.zip".to_owned(), 5_000),
            ("notes.txt".to_owned(), 100),
        ];
        let plan = cap_removals(&entries, "W", 1_000);
        assert_eq!(plan.remove, ["W_1.zip"], "nothing foreign is deleted");
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
}
