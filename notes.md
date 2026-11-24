# Simple Backups — port notes

Ported from **Simple Backups 1.21-4.0.30** (`ChaoticTrials/SimpleBackups`,
**Apache-2.0**) for Minecraft Java 1.21.1 NeoForge. Server-only.

Status: **implemented; verified in game at 0.1.0** — 15 runtime tests, `stage → check → test →
build` green, and the backup scenarios pass (see below). **Republished as the SDK package
0.2.0** on 2026-11-18: 23 tests, clippy/fmt clean, `check`/`fits` green, artifact reproducible.
**0.2.1** fixed the registry review's four findings and re-ran the leg in a launcher-built game.
**0.2.2** (2026-11-21) is the disk-cap slice: `max_disk_size` (upstream `maxDiskSize`, 25 GB
default) prunes the oldest archives after a good backup, repinned to Horizon `0a875da`; 36 tests,
`check`/`fits` green, and the new cap scene (with its counterfactual) passed in game.
**0.2.3** (2026-11-23) is the review fix for the cap: retention now deletes only archives the port
can identify as its own (an archive manifest), skips both passes after a stale copy, and orders
same-second generations numerically; 46 tests, scenarios E/F and the stale selftest in game.
**0.2.4** (2026-11-23) is review round two: a marker-less archive is deletable only after an
operator's `/simplebackups adopt <file>`, a new backup is allocated above the highest existing
same-second suffix (and never deleted by the pass that follows), and a malformed name can no longer
panic the server tick; 53 tests, scenarios G/H/H2 in game, and the pre-fix behaviour kept as
`negative` legs.
**0.2.5** (2026-11-24) is the pause/unpause slice: two level-2 commands flip a flag the tick
already reads. The flag persists beside the world (upstream's `BackupData.paused`), pausing
suppresses only the schedule, and a manual backup still works; 56 tests, scenario I green with a
`SELFTEST=pause` inversion, and `Compiling simplebackups (1 of 1)` on the launcher build.
**0.2.6** (2026-11-24) is the persisted-schedule slice: the tick progress toward the next backup is
written beside the world with a staged rename on each save and on close, and restored in
`ensure_world`, so a restart resumes the interval (upstream's `lastSaved` in `BackupData`); 59 tests,
scenarios J/J2 green with a `SELFTEST=ticks` inversion. The regex half of the gap stays out of scope
(R9 forbids a registry dependency). **0.2.7** (2026-11-24) added incremental chains and merge; 68
tests, scenario K. **0.2.8** (2026-11-24) is the empty-server policy (`noPlayerBackups` /
`noPlayerBackupCount`): a pure gate wrapping `shouldRunBackup`, one persisted counter beside the
world and two new `status` fields; 73 tests, scenarios L/L2, with the playerless branch proven
unit-first because a `ctl` launch always has its owner online.
**0.2.9** (2026-11-24) captures the game's `logs/latest.log` into every archive as a flat
`backup.log` entry (upstream `captureLatestLog`, default true): one config key, one pure entry-name
rule and one guarded read after the world walk; 77 tests, scenarios M/M-off, `SELFTEST=latestlog`,
and all 14 e2e logs green under `--strict`.
**0.2.10** (2026-11-24) is the empty-server interval (`no_player_backup_timer`, upstream
`noPlayerBackupTimer`, default 0 → reuse the main interval): one config key, one pure
`empty_period_interval` rule, one branch in the existing wall-clock due gate and a `timers=` field
on the existing `saves=` status line; 80 tests, scenario N with `SELFTEST=noplayertimer`, and all
15 e2e logs green under `--strict`. **0.2.11** (2026-11-24) binds an adoption to the archive's
SHA-256 and re-checks that digest immediately before retention deletes a file, so a replacement
under the same name and size needs re-adoption; 84 tests and a 16th e2e leg with its own inversion.

**Class + tier** (from `ports/ledger.csv`, per `horizon/docs/modpacks.md`):
**server-only / logic**. The approach is the **logic shape**: a pure policy
crate (`runtime.rs`), server lifecycle/tick hooks in the injected support
module, and `/simplebackups` commands; there is no carried content and no
`.hmod`.

## Behaviour (slice 1)

- **Schedule**: `interval_ticks > 0` counts server ticks; otherwise
  `interval_minutes` of wall clock (0 disables the schedule). Due checks run at
  `ServerTickEnd` of a running tick.
- **Save**: `Game::save_game()` (flushes chunks, parked levels, `level.dat`,
  player data and clocks) runs on the server thread before the copy.
- **Backup**: a worker thread walks the world folder into
  `<world>/backups/<world>_<UTC>.zip.part`, deflates, finishes, `sync_all`s and
  renames. `.part`/`.tmp`, `session.lock`, symlinks and the backups subtree are
  skipped; entries are `<world>/<relative>`.
- **Retention**: after a successful rename, only `<world>_*.zip` files are
  pruned oldest-first until `keep` remain (`keep` clamped to 1). Foreign files
  and `.part` files are untouched.
- **Commands** (level 2): `/simplebackups backup [quiet]`, `status`, `list`.
  A manual backup ignores `enabled`; it queues and starts at the next
  `ServerTickEnd`.
- **Config** (`config/simplebackups.toml`): `enabled`, `interval_minutes`,
  `interval_ticks`, `keep`, `compression_level`, `save_before_backup`,
  `broadcast`, `backups_dir`, `ignored_paths`, `ignored_files`. A missing file
  is written with the defaults; bad values keep their default and log.

## Hooks

| Hook | Helper | What it does |
|---|---|---|
| `server_about_to_start` (v1, API 44) | `about.rs` | Load or create the config. |
| `world_lifecycle` (v1, API 44) | `world.rs` | Remember/forget the world folder. |
| `server_tick` (v1) | `tick.rs` | Poll the worker; start due/manual backups. |
| `server_stopping` (v1, API 44) | `stopping.rs` | No new backups. |
| `register_commands` (v1, API 8) | `register.rs` | Declare the commands. |
| `perform_command` (v1, API 8) | `perform.rs` | Queue/answer them. |

Skills: superpowers/using-superpowers,verification-before-completion; BeautifulRust/rust-best-practices

## Review fixes (2026-11-18) — registry review #1259

| Finding | Fix | Test / proof |
|---|---|---|
| `src/runtime.rs:114` — `backups_dir` accepted absolute paths and parent traversal, and a symlink could redirect the writes/deletes (support.rs:451, :553) | `safe_backups_dir` normalises the setting (`.` dropped, `..`/absolute refused, empty → default) and `owned_dir` canonicalises the folder and refuses one that resolves outside the world before any write or prune | `runtime::tests::{a_backup_folder_may_not_leave_the_world, an_unsafe_backups_dir_is_refused_and_the_default_kept}`, `support::tests::a_backups_folder_that_resolves_outside_the_world_is_refused` |
| `src/support.rs:500` — `File::create` followed a pre-existing `.zip.part` link and truncated its target | the staging entry is removed (a link itself, not its target) and re-created with `create_new(true)` | `support::tests::a_planted_staging_symlink_is_not_followed_or_truncated` (watched failing with the old `File::create`) |
| `src/runtime.rs:286` — the walk normalised the visited path but compared it with the raw configured output prefix, so `./backups` was copied into the next archive | `should_skip` normalises the prefix with the same `normalise` (and the glue passes the **resolved** folder's relative path), so the real output subtree is always excluded | `runtime::tests::a_dot_spelled_output_folder_is_still_excluded_from_the_zip` |
| `Cargo.toml:11` / licence — the SHA-bound package said MIT while the registry and upstream say Apache-2.0 | the outbound licence is **resolved to Apache-2.0**: `LICENSE` is the Apache text, `Cargo.toml` and `horizon-mod.toml` declare it, and `LICENSE-NOTICE.md`/`README.md` say so | `unzip -p simplebackups.hmod horizon-mod.toml`/`Cargo.toml`; the entry's fields are the orchestrator's |

The port-local `Config` type was renamed `Settings` (its name collided with the SDK's `Config`,
which `fits` flagged as a changed item on the newest Nightly); behaviour is unchanged.

**Found while proving the leg:** the zip writer read each file **twice** (a checksum pass, then
a copy), so a chunk the game wrote in between produced an entry whose CRC failed on read-back
(`check_zip`'s `testzip`). It now copies in one pass with a data descriptor (flag bit 3) and
the CRC always matches the bytes stored. The save-overlap scenario can only be overlapped by a
*second* world save, which single player cannot produce (no `/save-all`; the administration
profile is the dedicated server's, and pause/resume does not save here) — the rule is pinned by
`support::tests::an_overlapped_copy_is_redone_only_within_the_rule` and the e2e prints a `SKIP`
line for the two overlap runs rather than asserting on a save it cannot make. **Ladder:** 29
tests, clippy `-D warnings` 0, `fmt --check` clean, two builds byte-identical, `check` and `fits`
exit 0; `launcher-game.sh run simplebackups` → `PASS` + `EXIT:0`, `GAME=$CONTROL_GAME` →
`EXIT:1` (`../_shared/evidence/simplebackups-e2e/`).

Package 0.2.1: **35,073 B**, SHA-256 `6565f5acd73b5c03f2f49f3009b8a520cf945b25daec1f08d9f7869448b978a2`;
a second build is byte-identical (`cmp`), `check` and `fits` exit 0.

## Slice 2 (0.2.2, 2026-11-21) — the disk cap (`max_disk_size`)

**Gap it closes** (this file, *Gaps / deviations* → deferred, and *Gaps after the migration* item 4:
"… a disk cap"). Upstream's `maxDiskSize` (`CommonConfig.java:80-83`, parsed by
`StorageSize.java:32-37`, enforced by `BackupThread.java:154-178`): after a good backup, delete the
oldest files until the folder is under the cap; if only one file is left, report an error and stop.
At 0.2.1 the only bound on disk use was `keep`, so ten 25 GB archives could fill a volume.

**Pin moved with this slice** (the 2026-11-21 repin): `[native] horizon` `0b14f8a` → **`0a875da`**,
mixins unchanged (`e59da909`), `api = "0.2"`. `.horizon/sdk` was deleted and re-fetched so the
build really used the new surface (`SDK 0a875da (build 0a875da9…) in ./.horizon/sdk`) — the trap
that bit `hostilenetworks`; repinning the manifest alone does not refresh the cached SDK.

**Rules (pure, `src/runtime.rs`).** `parse_size` accepts upstream's `"<n> <unit>"` with binary
`B/KB/MB/GB/TB` (case- and space-insensitive, a bare number is bytes, a non-integer or an
overflowing value is refused); `format_size_setting` writes the largest exactly-dividing unit, so
the default file carries `max_disk_size = "25 GB"` and the key round-trips. `cap_removals` counts
**every regular file in the folder** (upstream's folder size) but its removal set is only this
world's `<world>_*.zip`, oldest name first (the names are UTC timestamps), and it always stops with
one archive left; `over` is the "still over the cap" state upstream reports and stops on.

`runtime.rs` is therefore **no longer byte-identical to the retired bundle** (the 0.2.1 review fixes
had already changed it; this slice adds the cap): sha256 `150a2d7e…`.
`src/{lib,support,zip}.rs` are unchanged in shape; `src/support.rs` gained the glue.

**Glue.** `publish` enforces the cap after the `keep` prune (`support::enforce_cap`), records the
folder's size and the `over` flag on the state, and `/simplebackups status` gained one line —
`disk=<size> cap=<size> over=<bool>` — after the four lines that stay byte-identical, so the older
scenario assertions still match. The completion broadcast appends `; removed N over the disk cap`
and, when one archive is left and it is still too big, `; still over the disk cap (… of …)`.

**Deliberate deviations (safety, recorded).** The cap never deletes a foreign file — only
`<world>_*.zip`, the same rule `keep` already had — while upstream deletes every file in the folder;
it counts foreign files toward the total, so no bytes are hidden from the cap. Deletion still happens
after a successful publish, never before the copy (the 0.1.0 rule).

**Red → green.** The five cap rules were written as failing tests first, against stubs that returned
nothing; verbatim (`sdk-test-red.log`):

```
---- runtime::tests::the_cap_removes_the_oldest_archives_first_but_never_the_last stdout ----
thread 'runtime::tests::the_cap_removes_the_oldest_archives_first_but_never_the_last' (3104017) panicked at src/runtime.rs:684:9:
assertion `left == right` failed
  left: []
 right: ["W_1.zip", "W_2.zip"]

failures:
    runtime::tests::an_unparsable_disk_cap_keeps_the_default_and_warns
    runtime::tests::sizes_are_parsed_with_binary_units
    runtime::tests::sizes_render_in_the_largest_exact_unit
    runtime::tests::the_cap_counts_the_folder_but_only_deletes_our_archives
    runtime::tests::the_cap_removes_the_oldest_archives_first_but_never_the_last
    runtime::tests::the_disk_cap_defaults_to_upstreams_25_gb

test result: FAILED. 29 passed; 6 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

The in-game assertion is scenario D of `tools/e2e.sh`, fail-capable twice over: a **1 B** cap with
`keep = 3` must leave exactly the **newer** archive and report `over=true`, while the same scene
with a **1 TB** cap must leave **both** (the counterfactual). A package without the cap keeps two in
both runs and fails the first assertion.

**Measured in game** (launcher-built game `native-0a875da-1c11ca957601`, sources byte-identical to
this tree — `../_shared/evidence/simplebackups-e2e/build-record.txt`):

```
cap 1 B: first backup …/SimpleBackupsCap_2026-11-21_09-10-27.zip
status: disk=10177137 B cap=1 B over=true saves=0 retries=0 stale=false in_flight=false last=SimpleBackupsCap_2026-11-21_09-10-38.zip …
zip entries: 20
cap enforced: the oldest archive was removed, …/SimpleBackupsCap_2026-11-21_09-10-38.zip kept
list: SimpleBackupsCap_2026-11-21_09-10-38.zip (9.7 MB) …
== scenario D counterfactual: a 1 TB cap keeps both ==
status: disk=18289807 B cap=1 TB over=false … last=SimpleBackupsCapOff_2026-11-21_09-11-05.zip …
counterfactual: a 1 TB cap keeps both backups
list: SimpleBackupsCapOff_2026-11-21_09-10-54.zip (7.7 MB) SimpleBackupsCapOff_2026-11-21_09-11-05.zip (9.7 MB) …
PASS: …
EXIT:0
```

Evidence: `../_shared/evidence/simplebackups-e2e/e2e-launcher.log` (`PASS` + `EXIT:0`),
`…-control.log` (`EXIT:1`, the unmodded game makes no backup), `…-selftest.log` (`EXIT:1`),
`check-e2e-logs.py simplebackups-e2e --strict` → `logs=3 ok=3`.

The run was made from this tree; the only later edit to the package is the `README.md` sentence that
documents the new key, so the compiled `src/*.rs` and the manifest the game built are byte-identical
to the delivered package (`build-record.txt` compares them).

### Package 0.2.2

| | |
|---|---|
| version | **0.2.2** (`horizon-mod.toml` + `Cargo.toml`) |
| bytes | **38,169 B** |
| sha256 | `2edec94439b83c8d030786f737a27e9edfee2649996a81b8dd28bacc96bf8661` |
| entries | **10** — `horizon-mod.toml`, `Cargo.toml`, `LICENSE`, `LICENSE-NOTICE.md`, `README.md`, `src/{lib,runtime,support,zip}.rs`, `uses.json`; **no `Cargo.lock`** |
| reproducible | second build after `rm -f Cargo.lock`: `cmp` identical, same sha |
| `check` | `simplebackups 0.2.2 (native-mod, format native-1/0a875da…/e59da90…, for 26.3)`, exit 0 |
| `fits --build 0a875da…` | `0a875da  made for` (the pin tautology, kept as the pin line) |
| `fits --build 0b14f8a…` | `0b14f8a  fits` — the real gate, exit 0 |
| tests | **36 passed, 0 failed** — 25 `runtime`, 4 `support`, 7 `zip` |

`fits` is run with the full 40-hex commit: a short name makes the tool list the GitHub releases
(unauthenticated, `403` in this window), while a full commit resolves from `.horizon/cache`.

### Coverage (family → state)

The family column is prose on purpose: this package carries no content, so
`check-coverage.py simplebackups` has no ids to inventory (`entries=0`) and reports
`missing_families=0`; the rows below are the jar's behaviour families, and each `tested` row names the
phase that asserts it.

| Family | Status | Evidence |
|---|---|---|
| scheduled backup (tick or wall clock) | tested | scenario A: `interval_ticks=40` → a zip; scenario B: `enabled=false` → none in 100 ticks |
| save before copy | tested | every scenario: `LevelRef::request_save` → `world_saved` → the copy starts (`status` `saves=` grows) |
| zip of the world folder | tested | scenario A/B/D: `zipfile.testzip()` CRC pass, `level.dat` inside, no `session.lock`, no `backups/` subtree |
| retention (`keep`) | tested | scenario A: `keep=1` leaves exactly one archive; scenario E: the older **mod-made** archive is removed while a hand-made ZIP with the same prefix stays |
| disk cap (`max_disk_size`) | tested | scenario D: `1 B` leaves the newer archive and `over=true`; `1 TB` counterfactual leaves both and `over=false`; scenario F: the cap deletes nothing after a stale copy; scenario H2: the cap removes the older same-second generations and never the archive just written |
| archive identity (own manifest) | tested | `support::tests::an_archive_identifies_itself_not_by_its_name`; every `check_zip` call parses `simplebackups.json` and asserts `format` + `world` |
| legacy archive adoption (`/simplebackups adopt`) | tested | scenario G: a hand-made export with the world's own shape survives both passes, `adopt` answers `adopted`, `status` reports `adopted=1`, and the next publish prunes it; `support::tests::an_adopted_legacy_zip_becomes_deletable`; the 0.2.3 package deletes the export without any adoption (`e2e-launcher-adopt-negative.log`, `EXIT:1`) |
| adoption digest binding (0.2.11) | tested | scenario G's replacement: the adopted export is rewritten under the same name **and size** with different bytes, survives the next publish, then is re-adopted and pruned by the one after; `SELFTEST=adopt-digest` inverts the survival assertion (`EXIT:1`). Pure: `runtime::tests::{the_adoption_journal_round_trips_the_digest,an_adoption_whose_digest_no_longer_matches_refuses_deletion,a_matching_digest_still_permits_deletion,the_digest_matches_the_standard_sha256_vectors}`; glue: `support::tests::a_replaced_adopted_archive_survives_until_it_is_re_adopted` |
| stale copy skips retention | tested | scenario F: `stale=true` with `keep=1` and a `1 B` cap leaves both archives and `over=true`; `STALE_SELFTEST=1` asserts the pre-fix defect and exits 1 |
| same-second generations | tested | `runtime::tests::{same_second_backups_sort_by_generation_not_as_text,the_cap_also_orders_same_second_generations_numerically,a_pruned_suffix_is_never_reused_for_a_new_backup}`; scenario H: a backup in a second that already has `-41`/`-42` becomes `-43` and both `-42`/`-43` survive `keep=2`; H2: the cap keeps only `-43`. The 0.2.3 package writes the base name and deletes it in the same pass (`e2e-launcher-same-second-negative.log`, `EXIT:1`) |
| malformed archive name | tested | `runtime::tests::a_name_with_a_multibyte_character_at_the_stamp_boundary_is_refused`: a two-byte character at byte 19 is refused, not sliced (the pre-fix `split_at(19)` panicked) |
| commands `backup` / `status` / `list` / `adopt` | tested | `ctl chat "/simplebackups backup"` and `status` in every scenario, `list` in scenario D: the reply names every archive in the folder, and after a `1 B` cap it no longer names the deleted one; `adopt` in scenario G (confirmed reply, `adopted=1`, and a refusal for a name that is not this world's) |
| save-overlap retry (`retry_on_save`) | tested | unit test `should_retry`; scenario F forces a real overlap with `ctl save` during a 256 MiB copy (`stale=true`, `saves=6`); scenario C's pause/resume still prints `SKIP` (that path cannot make a save) |
| `pause` / `unpause` | tested | scenario I: `interval_ticks=40`, a first zip (differential), `/simplebackups pause` → 150 ticks with no new zip and `status` `paused=true` (the `saves=` line, not the whole chat buffer), the persisted flag file read back, a manual `backup` while paused → a fresh zip, `/simplebackups backup unpause` → `paused=false` and exactly one new zip named by `list`; `SELFTEST=pause` (`EXIT:1`) proves the paused-window assertion can fail. The pure rules are `runtime::tests::a_paused_schedule_never_starts_a_backup` and `the_pause_flag_round_trips_through_its_file`, the file round-trip `support::tests::the_pause_flag_is_persisted_beside_the_world` |
| empty-server policy (`noPlayerBackups` / `noPlayerBackupCount`) | tested | scenario L: `status` reports `players=1` (the harness owner is always online), so the online branch's scheduled backup is asserted and the playerless window is recorded as not producible; scenario L2 runs the same with `no_player_backups = true`; the empty branch and the zero/count cap are proven by `runtime::tests::{an_empty_server_never_schedules_by_default,no_player_count_caps_backups_per_empty_period}`, the file round-trip by `support::tests::the_no_player_counter_is_persisted_beside_the_world` |
| empty-server interval (`noPlayerBackupTimer`) | tested (unit branches + parsed status) | `runtime::tests::{an_explicit_empty_timer_replaces_the_main_interval,a_zero_empty_timer_falls_back_to_the_main_interval,the_no_player_timer_key_defaults_to_zero_and_round_trips}` prove both branches red-first; scenario N parses the **newest** `saves=` line and asserts `timers=7` from `no_player_backup_timer = 7`, `SELFTEST=noplayertimer` inverts that one value to `EXIT:1`. A genuinely playerless interval is **not producible** (`players=1` always), so neither branch is dressed up as a chat command |
| `latest.log` → `backup.log` | tested | scenario M: a concrete line is read from the scratch `<home>/logs/latest.log` before `ctl chat "/simplebackups backup"`; the newest archive's flat `backup.log` entry contains that **exact** line (`python3 zipfile`, CRC checked by `check_zip`); the `capture_latest_log = false` counterfactual packs no such entry and `SELFTEST=latestlog` inverts to `EXIT:1`. Pure rules: `backup_log_entry` (one flat name) and `game_log_path` (no absolute/`..`/other file); the glue refuses symlinks and canonical paths outside the game home. |
| disk reservation before a copy | blocked(no-disk-space-api) | pin `0a875da` has no free-space query: std has none, `horizon-mod-api` has none, and R9 forbids the crate that would supply one — see Slice 10 and `_shared/evidence/simplebackups/reserve-gate-0a875da.log` |
| persisted schedule counter (tick mode) | tested | scenario J: a zip before quit, the state file beside the world confirmed and seeded to `ticks = 10`, `ctl quit` + relaunch of the same world, `ticks=<restored ≥ 10>` parsed from the newest status line, an empty window sized from the restored value, then a fresh zip (`check_zip` CRC + manifest); `SELFTEST=ticks` (`EXIT:1`) inverts the restored value; pure rules `runtime::tests::{the_persisted_tick_counter_round_trips_through_its_file, a_restored_counter_resumes_the_interval}`, file round-trip `support::tests::the_schedule_counter_is_persisted_beside_the_world`; scenario J2 is the fresh-world differential (no state file, `ticks=0`, needs the full interval) |
| partial / chains / merge | tested | scenario K: an incremental child names its full base, carries only `World/changed-marker.txt`, `status` reports `chain=2`, `list` names both, `merge` writes a standalone archive; the fresh-world first backup is full with `chain=1`; `SELFTEST=chain` (`EXIT:1`) inverts the base assertion. Pure rules: `entry_changed`, `choose_kind`, `changed_threshold`, `full_timer_due`, `merge_order`, the chain-state and manifest round-trips. |
| regex ignores | not ported | R9 forbids the `regex` crate a registry dependency would need; implemented without one would mean re-writing a matcher here, deferred |
| sync-copy mode (`copy_on_server_thread`) | blocked(no-sync-observable) | the harness cannot see the copy in one `ctl` round trip: `ctl chat backup` answers in 44 ms with `{"ok":true}` while a 128 MiB archive appears 10.6 s later and the immediate `list` (379 ms) runs before the backup tick (`ticks=42`, `in_flight=false`, 0 archives) — see Slice 11 and `_shared/evidence/simplebackups/sync-copy-gate-0a875da.log` |

## Slice 3 (0.2.3, 2026-11-23) — review fix: retention's identity, a stale copy, same-second order

**Review.** `NikitaMGrimm` reviewed round one at `5cfa312aeee8`: the archive matched the listing and
`check` passed, but the new disk cap could (a) leave a backup that **overlapped a save** as the only
restore point and (b) delete **manually retained ZIPs whose names match the world prefix**, even below
`keep`. Both came from the same root cause: retention's idea of "a backup" was the *name*
`<world>_*.zip`, and it ran unconditionally after every publish.

**Gap line.** *Gaps after the migration* item 4 / *Deliberate deviations* in this file: "The cap never
deletes a foreign file — only `<world>_*.zip`, the same rule `keep` already had". A name prefix is not
an identity, and a copy a save overlapped is not a restore point.

**How an archive is identified as the mod's own.** Every archive the port writes carries its own
`simplebackups.json` entry (the first entry, stored, `src/runtime::manifest`):

```json
{"format":"simplebackups/1","world":"World","backup":"World_2026-11-23_12-28-20.zip"}
```

Retention reads that entry from the archive (`zip::read_entry`: central directory → local header →
stored bytes) and treats the file as this world's backup only when `format` **and** `world` match.
Archives written before the marker existed (0.2.0–0.2.2) are recognised by their world-zip shape
instead — every entry under `<world>/` and `<world>/level.dat` present — so an upgrade does not leave
its old archives undeletable. A ZIP that matches the name prefix but has neither is never deleted, and
`runtime::parse_archive` additionally refuses a name that is not
`<world>_<YYYY-MM-DD_HH-MM-SS>[-<generation>].zip`.

**Rules (pure, `src/runtime.rs`).** `parse_archive` parses the UTC stamp (fixed separators, 19 bytes)
and the optional `-<n>` generation; `chronological` orders by `(stamp, generation)` so `-10` follows
`-9` and the base name (generation 0) is the oldest of its second; `prune` and `cap_removals` take the
**identified** archives only, and both stop before the newest one. `cap_removals` still counts every
regular file in the folder toward the size, so a foreign file cannot hide bytes from the cap.

**Glue (`src/support.rs`).** `archive_is_ours` is the identity test; `owned_archives` builds the
retention candidates; `publish` skips **both** passes when the published copy is stale, reports the
folder's size and `over=true`, and the completion message says `retention skipped (a save overlapped
this copy)`. The stale copy stays on disk (it is the only copy of that moment) but never causes a
confirmed restore point to be deleted.

**Red → green.** Four failing tests first, against the 0.2.2 code; verbatim from `sdk-test-red.log`:

```
---- runtime::tests::same_second_backups_sort_by_generation_not_as_text stdout ----
assertion `left == right` failed: the oldest nine generations go, not the ones that sort first as text
  left: ["W_2026-11-03_12-34-56-1.zip", "W_2026-11-03_12-34-56-10.zip", "W_2026-11-03_12-34-56-2.zip", …, "W_2026-11-03_12-34-56-8.zip"]
 right: ["W_2026-11-03_12-34-56.zip", "W_2026-11-03_12-34-56-1.zip", "W_2026-11-03_12-34-56-2.zip", …, "W_2026-11-03_12-34-56-8.zip"]

---- runtime::tests::the_cap_also_orders_same_second_generations_numerically stdout ----
assertion `left == right` failed: the cap must delete generations 0-8, never the numerically newest
  left: ["W_2026-11-03_12-34-56-1.zip", "W_2026-11-03_12-34-56-10.zip", …]
 right: ["W_2026-11-03_12-34-56.zip", "W_2026-11-03_12-34-56-1.zip", …]

---- support::tests::a_foreign_zip_named_like_a_backup_survives_retention stdout ----
a zip the mod did not write is not a backup and must survive

---- support::tests::a_stale_copy_skips_both_retention_passes stdout ----
keep must not run after a stale copy: ["World_2026-01-01_00-00-01.zip"]

test result: FAILED. 36 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

Green: **46 passed, 0 failed** (28 `runtime`, 7 `support`, 11 `zip`) — `sdk-test.log`.

**In game (launcher-built game `native-0a875da-a2b71884a7ff`, sources byte-identical to this tree —
`../_shared/evidence/simplebackups-e2e/build-record.txt`).** New scenario E places a hand-made ZIP
`SimpleBackupsForeign_1999-01-01_00-00-00.zip` (entry `my-own-notes.txt`) in the folder, then takes two
backups with `keep=1` and `max_disk_size="1 B"`:

```
hand-made archive: …/SimpleBackupsForeign_1999-01-01_00-00-00.zip
first mod backup: …/SimpleBackupsForeign_2026-11-23_12-27-58.zip
zip entries: 21                         # the manifest + the world
unmanaged archive survived; the older mod archive was removed (the differential)
```

New scenario F runs with `save_before_backup=false`, takes one clean backup, adds a 256 MiB payload,
starts a second copy and calls `ctl save` six times while it is being written:

```
clean restore point: …/SimpleBackupsStale_2026-11-23_12-28-16.zip
status: disk=279251496 B cap=1 B over=true saves=6 retries=0 stale=true in_flight=false last=SimpleBackupsStale_2026-11-23_12-28-20.zip …
stale copy published; both retention passes skipped: …/SimpleBackupsStale_2026-11-23_12-28-16.zip survives beside it
```

`STALE_SELFTEST=1` runs the same scene and asserts the **pre-fix** behaviour (retention ran, one archive
left); it exits 1 either way and says which it saw — with this package:

```
FAIL: STALE_SELFTEST: retention was skipped after a stale copy (2 archives, …/SimpleBackupsStaleSelf_… kept) — the pre-fix defect is gone, so the assertion is alive
EXIT:1
```

Evidence: `../_shared/evidence/simplebackups-e2e/e2e-launcher.log` (`PASS` + `EXIT:0`),
`…-control.log` (`EXIT:1`, the unmodded game makes no backup), `…-selftest.log` (`EXIT:1`),
`…-stale-selftest.log` (`EXIT:1`); `check-e2e-logs.py simplebackups-e2e --strict` → `logs=4 ok=4`.

### Package 0.2.3

| | |
|---|---|
| version | **0.2.3** (`horizon-mod.toml` + `Cargo.toml`) |
| bytes | **45,307 B** |
| sha256 | `4c9061c0e2c5f5f2baec76a14b764293655120522f16992b90f133a0aa5c592a` |
| entries | **10** — `horizon-mod.toml`, `Cargo.toml`, `LICENSE`, `LICENSE-NOTICE.md`, `README.md`, `src/{lib,runtime,support,zip}.rs`, `uses.json`; **no `Cargo.lock`** |
| reproducible | second build after `rm -f Cargo.lock`: `cmp` identical, same sha |
| `check` | `simplebackups 0.2.3 (native-mod, format native-1/0a875da…/e59da90…, for 26.3)`, 45,307 bytes, exit 0 |
| `fits --build 0a875da…` | `0a875da  made for` (the pin tautology, kept as the pin line) |
| `fits --build 0b14f8a…` | `0b14f8a  fits` — the real gate, exit 0 |
| tests | **46 passed, 0 failed** — 28 `runtime`, 7 `support`, 11 `zip` |
| `verify-sdk-package.py` | `1/1`, `package matches the tree` |

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices, rust-review

### Changelog (0.2.3)

> **0.2.3 — retention only deletes the port's own archives, and never after a stale copy.** Every
> backup now carries a `simplebackups.json` manifest (`format = simplebackups/1`, the world name), and
> `keep`/`max_disk_size` delete only archives whose manifest identifies them (0.2.0–0.2.2 archives are
> recognised by their `<world>/…` + `level.dat` shape). A hand-made ZIP that matches the world prefix is
> never deleted, however old its name. When a copy is published `stale=true` (a save overlapped it),
> **both** retention passes are skipped so the previous confirmed restore point stays — the folder may
> report `over=true` until the next good backup. Same-second backups are ordered by their numeric
> generation (`-10` after `-9`), so the newest one is never treated as the oldest. 46 tests; proven in
> game by scenario E (hand-made archive survives, older mod archive removed) and scenario F (stale copy
> leaves the clean one).

## Slice 4 (0.2.4, 2026-11-23) — review round two: adoption, allocation, a checked name

**Review.** `NikitaMGrimm` reviewed 0.2.3 and accepted the stale-copy fix; four findings remained,
three of them code:

1. *"Require an operator to adopt legacy ZIPs before retention can delete them. A manual export can
   have the expected filename and contain the same world files."* The 0.2.3 fallback recognised a
   marker-less archive by its `<world>/…` + `<world>/level.dat` shape — which a hand-made export
   has by construction.
2. *"Allocate a same-second generation above the highest existing suffix, and protect the newly
   published clean archive in both retention passes. Reusing a pruned suffix can make the new
   backup sort behind an older one and be deleted immediately."* 0.2.3 took the **lowest free**
   suffix, so a folder holding only `-3`…`-5` made the new backup `-0`: the oldest of its second,
   and the pass that runs right after deleted it.
3. *"Use a checked split or validate the timestamp before slicing it. A malformed ZIP filename with
   a multibyte character at byte 19 can panic on the server tick."* `parse_archive` used
   `rest.split_at(19)` on an untrusted name: `byte index 19 is not a char boundary` is a panic, and
   a panic in `server_tick` is a crash.
4. (The fourth was the accepted stale-copy fix; nothing to change there.)

**Gap lines.** *Gaps / deviations*: "…only archives the port can identify as its own (a manifest
since 0.2.3; the world-zip shape for 0.2.0–0.2.2)" — a shape is not an identity; and the coverage
row "same-second generations | tested" named ordering but not allocation, so the rule that keeps a
new backup out of the removal set did not exist.

**What changed.**

- **`/simplebackups adopt <file>`** (level 2, the smallest workable operator action). It refuses
  anything but a bare file name inside the backup folder, a name that is not
  `<world>_<UTC>[-<generation>].zip` for this world, a file that is not a regular file, and a ZIP
  that is not world-shaped; it says so when the archive already carries the marker. It records
  `<bytes>\t<name>` in `simplebackups-adopted.txt` in the backup folder (written to a staging file
  and renamed), so **the adoption binds to the exact file**: replacing the archive under that name
  drops it out of retention again. `retainable` is now "the port's marker, or an adopted name +
  size **and** the world shape". `/simplebackups status` reports `adopted=<n>` on the disk line
  (the older `disk=/cap=/over=` assertions still match).
- **Allocation** (`backup_file_name`): the base name only when nothing of that second exists, else
  `max(generation) + 1` — `-6` after a surviving `-5`, `-4` when only the base and `-3` are there.
- **Protection**: `prune(archives, keep, protected)` and `cap_removals(entries, archives, cap,
  protected)` take the name the publish just wrote and never remove it; the cap may leave the
  folder over its limit rather than delete the backup that run just made.
- **`parse_archive`** takes the stamp with `str::get(..19)` / `str::get(19..)`: a non-boundary
  slice returns `None` instead of panicking.

**Red → green (two runs, verbatim from `sdk-test-red.log` / `sdk-test-red-adopt.log`).** The four
rules were written as failing tests first; 46 passed, 4 failed on the 0.2.3 code:

```
---- runtime::tests::a_name_with_a_multibyte_character_at_the_stamp_boundary_is_refused stdout ----
thread 'runtime::tests::a_name_with_a_multibyte_character_at_the_stamp_boundary_is_refused' (445934) panicked at /rustc/59807616e1fa2540724bfbac14d7976d7e4a3860/library/core/src/str/mod.rs:849:21:
end byte index 19 is not a char boundary; it is inside 'é' (bytes 18..20) of `2026-01-01_00-00-0é`

---- runtime::tests::a_pruned_suffix_is_never_reused_for_a_new_backup stdout ----
assertion `left == right` failed: the next generation is above the highest suffix, not the lowest free one
  left: "W_2026-11-03_12-34-56.zip"
 right: "W_2026-11-03_12-34-56-6.zip"

---- support::tests::a_manual_export_with_the_world_shape_needs_an_operator_to_become_deletable stdout ----
an archive no operator adopted must survive both retention passes, whatever its shape

---- support::tests::the_archive_just_written_survives_both_retention_passes stdout ----
the archive just written is never deleted by either pass (keep removed ["World_2026-01-01_00-00-00.zip"], cap removed [])

test result: FAILED. 46 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

The adopt command had its own red against the code that already had the other three fixes, with
`adopt_archive` stubbed (`Err("adopt is not implemented yet")`):

```
---- support::tests::an_adopted_legacy_zip_becomes_deletable stdout ----
adopt the export: "adopt is not implemented yet"

---- support::tests::an_archive_identifies_itself_not_by_its_name stdout ----
pre-marker archives are recognised by their world-zip shape

test result: FAILED. 51 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

Green: **53 passed, 0 failed** (32 `runtime`, 10 `support`, 11 `zip`) — `sdk-test.log`, clippy
`-D warnings` 0 warnings (`sdk-clippy.log`), `fmt --check` clean (`sdk-fmt.log`).

**In game (launcher build `native-0a875da-b49c3e979166`, `libsimplebackups.so` identity
`… simplebackups 0.2.4 1.99.1`; every packed file is the tree's bytes —
`../_shared/evidence/simplebackups-e2e/build-record.txt`).**

Scenario G — a manual export (the world prefix, a 1999 name, and `World/level.dat`,
`World/region/r.0.0.mca`, `World/data/raids.dat` inside) beside two mod backups, with `keep=1` and
a `1 B` cap:

```
manual export: …/SimpleBackupsAdopt_1999-01-01_00-00-00.zip
manual export survived both passes: …/SimpleBackupsAdopt_1999-01-01_00-00-00.zip
adopt reply: adopted SimpleBackupsAdopt_1999-01-01_00-00-00.zip (456 B); retention may now delete it
adopted export pruned by the next publish (the differential)
```

Scenario H — a second that already holds `-41` and `-42` (copies of a real archive, so they carry
the marker) and no base name, `keep=2`, cap `1 TB`:

```
seeded second: SimpleBackupsSameSecond_2026-11-23_17-40-41 (attempts: 1)
new archive: SimpleBackupsSameSecond_2026-11-23_17-40-41-43.zip
status: … last=SimpleBackupsSameSecond_2026-11-23_17-40-41-43.zip …
both archives of the same second survive, -42 then -43
```

Scenario H2 — the same seeding with `keep=3` and a `1 B` cap: the cap removes `-41` and `-42` and
**keeps the archive it just wrote** (`last=…-43.zip`, `over=true`).

**The two scenes are fail-capable against the previous package.** Both were run on the exported
0.2.3 game before rebuilding (`ADOPT_NEGATIVE=1`, `SAME_SECOND_NEGATIVE=1`; that game's record is
`../_shared/evidence/simplebackups-e2e/build-record-0.2.3.txt`):

```
FAIL: ADOPT_NEGATIVE: the package deleted a manual export without an operator adoption — the scene fails on it, so it is alive
EXIT:1
```

```
FAIL: SAME_SECOND_NEGATIVE: the publish wrote SimpleBackupsSameSecondNeg_2026-11-23_17-33-21.zip and deleted it in the same pass (the reused base suffix) — the scene fails on it, so it is alive
EXIT:1
```

The second line is the data-loss race itself: the archive that publish wrote was allocated the free
base name, sorted behind `-41`/`-42`, and was one of the two the prune removed.

Every earlier leg stays green: `PASS` + `EXIT:0` (`e2e-launcher.log`, scenarios A/B/C×2/D×2/E/F/G/H/H2),
control `EXIT:1`, `SELFTEST=1` `EXIT:1`, `STALE_SELFTEST=1` `EXIT:1`; `check-e2e-logs.py
simplebackups-e2e --strict` → `logs=6 ok=6`.

### Package 0.2.4

| | |
|---|---|
| version | **0.2.4** (`horizon-mod.toml` + `Cargo.toml`) |
| bytes | **50,023 B** |
| sha256 | `f7a25eaef989cda98400e5f9eb7a44cacfe58c2f0b40cb8c27c30b2b824733c5` |
| entries | **10** — `horizon-mod.toml`, `Cargo.toml`, `LICENSE`, `LICENSE-NOTICE.md`, `README.md`, `src/{lib,runtime,support,zip}.rs`, `uses.json`; **no `Cargo.lock`** |
| reproducible | second build after `rm -f Cargo.lock`: `cmp` identical, same sha |
| `check` | `simplebackups 0.2.4 (native-mod, format native-1/0a875da…/e59da90…, for 26.3)`, 50,023 bytes, exit 0 |
| `fits --build 0a875da…` | `0a875da  made for` (the pin tautology, kept as the pin line) |
| `fits --build 0b14f8a…` | `0b14f8a  fits` — the real gate, exit 0 |
| tests | **53 passed, 0 failed** — 32 `runtime`, 10 `support`, 11 `zip` |
| `verify-sdk-package.py` | `1/1`, `package matches the tree` |

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices, rust-review

### Changelog (0.2.4)

> **0.2.4 — a legacy archive needs an operator, a same-second backup gets the next suffix, and a
> malformed name can no longer panic.** `/simplebackups adopt <file>` (level 2) is now the only way
> a marker-less archive becomes deletable: the world-zip shape alone is what a hand-made export
> reproduces, so retention ignores such a file until an operator adopts that exact name and size
> (`simplebackups-adopted.txt` in the backup folder; `status` reports `adopted=<n>`). A new backup
> of a second that already holds `-N` is allocated `max+1` instead of the lowest free suffix, and
> the archive a publish just wrote is never removed by `keep` or by the disk cap — reusing a pruned
> suffix used to make the new backup sort behind an older one and be deleted in the same pass.
> `parse_archive` takes the timestamp with a char-boundary-checked slice, so a ZIP name with a
> multibyte character at byte 19 is refused instead of panicking the server tick. 53 tests;
> scenarios G, H and H2 are green, and the 0.2.3 package fails both (the `negative` legs).
> Package 50,023 B, sha256 `f7a25eae…`.

## Slice 5 (0.2.5, 2026-11-24) — `pause` / `unpause`

**Gap line.** The coverage table above: *"`pause` / `unpause` | not ported | gap (commands are
`backup`, `status`, `list` only)"*, and *Gaps after the migration* item 4. It fits **server-only /
logic**: two level-2 commands flip a boolean the tick already evaluates; no new hook, block entity,
menu, renderer or carried content. The jar has `commands/PauseCommand` and `network/Pause`.
`horizon/docs/modpacks.md` puts "backups" in the server-only class and "server events, typed
commands" in the logic tier, so this is the same shape as the existing command work.

**Upstream source (read again for this slice).** The checkout removed after the first analysis was
re-cloned read-only to `ports/_shared/upstream/simplebackups-src` at branch `1.21.1`, head
**`bf04de3e7b3e05504ee5cc974cf45833b0d4b9a4`** (the pack's `1.21-4.0.30` source), from
<https://github.com/ChaoticTrials/SimpleBackups>. Files read: `commands/PauseCommand.java`,
`commands/BackupCommand.java`, `BackupData.java`, `BackupThread.java:80-139`,
`network/Pause.java`, `client/ClientEventHandler.java`,
`src/main/resources/assets/simplebackups/lang/en_us.json`. The clone was deleted after these
findings were recorded.

**The pause flag's persistence rule, as read from the jar's source:**
`PauseCommand.java:20-26` registers `backup pause|unpause`; its `run` does
`BackupData.get(server).setPaused(this.paused)` (`:31-32`) and sends a `Pause` packet to all players.
`BackupData` is a `SavedData` held under the key `"simplebackups"` on the overworld's data storage
(`:36-42`); `paused` is written as the NBT boolean `"paused"` in `save` (`:29`) and read in `load`
(`:47`), and `setPaused` calls `setDirty()` (`:54-57`) — so **the flag persists across server
restarts**, loaded with the world's saved data. Only `shouldRunBackup` consults it
(`BackupThread.java:103-107`: `!isEnabled() || … || backupData.isPaused()`), so pausing suppresses
**scheduled** backups. The manual path is independent: `BackupCommand.java:34` calls
`BackupThread.createBackup`, which (`:126-129`) starts a thread with **no `paused` check**, so a
manual backup still runs while paused. `network/Pause.java` only drives a client-side red
"Backups paused" HUD string (`ClientEventHandler.java:14-37`; `en_us.json:7`); that is a `client`
half and is deliberately not ported.

**Port decisions.** `runtime.rs` gains the pure `schedule_allowed(enabled, paused)`,
`format_paused` and `parse_paused`; `PAUSE_FILE = "simplebackups-paused.txt"` lives beside the
world (not in the deletable backup folder), is read by `ensure_world` when the world opens (the
`SavedData` load moment) and written with a staged rename. `Core.paused` is appended to the
`saves=` status line. Semantics are otherwise upstream's: only the `due` gate consults the flag,
so a queued manual request still queues. Both upstream's `/simplebackups backup pause|unpause`
and the short `/simplebackups pause|unpause` are registered at level 2; feedback is "Backups
paused." / "Backups resumed.". A malformed or absent flag file means unpaused.

**Red → green.** Three failing tests first, against stubs that ignored the flag (`sdk-test-red-pause.log`):

```
---- runtime::tests::a_paused_schedule_never_starts_a_backup stdout ----
thread 'runtime::tests::a_paused_schedule_never_starts_a_backup' (29348) panicked at src/runtime.rs:781:9:
pausing suppresses only the schedule

---- runtime::tests::the_pause_flag_round_trips_through_its_file stdout ----
thread 'runtime::tests::the_pause_flag_round_trips_through_its_file' (29376) panicked at src/runtime.rs:788:9:
assertion `left == right` failed
  left: Some(false)
 right: Some(true)

---- support::tests::the_pause_flag_is_persisted_beside_the_world stdout ----
thread 'support::tests::the_pause_flag_is_persisted_beside_the_world' (29387) panicked at src/support.rs:1115:9:
the flag must survive being re-read from disk

test result: FAILED. 53 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

Green: **56 passed, 0 failed** (34 `runtime`, 11 `support`, 11 `zip`), clippy `-D warnings` 0
warnings, `fmt --check` clean — `sdk-test.log`, `sdk-clippy.log`, `sdk-fmt.log`.

**In game.** Launcher build `native-0a875da-15bed05fb2e0`, `Compiling simplebackups (1 of 1)`,
every packed source byte-identical to this tree (`../_shared/evidence/simplebackups-e2e/build-record.txt`).
Scenario I (the `list` and `status` assertions parse only the **newest** chat line — a grep over
the 20-message buffer matched a stale `last=<old archive>` and produced a false failure on the
first run, now fixed):

```
before pause: …/SimpleBackupsPause_2026-11-24_01-42-19.zip
paused line: saves=0 retries=0 stale=false paused=true
paused window: no new backup across 150 ticks
manual backup while paused: …/SimpleBackupsPause_2026-11-24_01-42-29.zip
paused line: saves=0 retries=0 stale=false paused=false
resumed backup: …/SimpleBackupsPause_2026-11-24_01-42-40.zip
list: SimpleBackupsPause_2026-11-24_01-42-40.zip (9.8 MB)
PASS: … pause suppresses only the schedule and persists beside the world, a manual backup still works while paused, and unpause resumes with exactly one new zip named by list (scenario I) …
EXIT:0
```

`SELFTEST=pause` inverts the paused-window assertion and ends
`FAIL: PAUSE SELFTEST: no new zip appeared during 150 paused ticks (the 40-tick interval would have made ~3) — the assertion is alive` + `EXIT:1`.
Control `EXIT:1`, `SELFTEST=1` `EXIT:1`, `STALE_SELFTEST=1` `EXIT:1`, `ADOPT_NEGATIVE=1` `EXIT:1`,
`SAME_SECOND_NEGATIVE=1` `EXIT:1`; `check-e2e-logs.py simplebackups-e2e --strict` →
`logs=7 ok=7 missing-exit=0 bad-exit=0`. The two older negative legs re-run on the 0.2.5 game exit
1 through the inconclusive guard because those fixes are present; their live differential against
0.2.3 is preserved in `build-record-0.2.3.txt` and the 0.2.4 notes.

**Remote build path.** Every cargo ladder step forwarded through
`ports/tools/build-slot.sh run --job G10-simplebackups --port ports/simplebackups --step …`,
printing `build-slot: forwarding … to the remote runner` on host **AdLf-PC** (`test`, `lint`,
`fmt`, `build` ×2, `check`, `fits` ×2). `BUILD_LOCAL=1` was never set. The launcher native build and
its seven game legs stayed local: the second box has the repaired SDK CLI and the 26.3 resources
but **no `horizon-launcher` and no Nightly versions cache** (checked over ssh), so a forwarded
game leg cannot build or launch. The launcher build was still run inside a slot lease with
`FORWARDED=1` (keep-local marker, not `BUILD_LOCAL`), producing the `Compiling simplebackups (1 of 1)` line above.

### Package 0.2.5

| | |
|---|---|
| version | **0.2.5** (`horizon-mod.toml` + `Cargo.toml`) |
| bytes | **51,968 B** |
| sha256 | `13fb3e0a397b1ecd8232ed338f1dc986c82770f6bc09055cedf7fca714b0e16b` |
| entries | **10** — `horizon-mod.toml`, `Cargo.toml`, `LICENSE`, `LICENSE-NOTICE.md`, `README.md`, `src/{lib,runtime,support,zip}.rs`, `uses.json`; **no `Cargo.lock`** |
| reproducible | two builds, each after `rm -f Cargo.lock`: identical sha and `cmp` |
| `check` | `simplebackups 0.2.5 (native-mod, format native-1/0a875da…/e59da90…, for 26.3)`, 51,968 bytes, exit 0 |
| `fits --build 0a875da91dbb0fdcb7ba8c42c11341aa229af51a` | `0a875da  made for` (the pin tautology) |
| `fits --build 0b14f8acfd97fa2c750e42795249dcc981178ca4` | `0b14f8a  fits` — the real gate |
| tests | **56 passed, 0 failed** |
| `verify-sdk-package.py simplebackups` | `1/1`, `package matches the tree` |

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices

### Changelog (0.2.5)

> **0.2.5 — pause/unpause, persisted beside the world.** `/simplebackups pause|unpause` (also
> `/simplebackups backup pause|unpause`, upstream's spelling) flips a flag the tick already reads,
> so pausing suppresses **only** the schedule — a manual `backup` still runs, as upstream's
> `createBackup` bypasses `shouldRunBackup`. The flag is stored in `simplebackups-paused.txt` beside
> the world and reloaded when it opens, mirroring upstream's `paused` boolean in its `BackupData`
> `SavedData` (which survives restarts). `/simplebackups status` reports `paused=<bool>`. The
> client-side red HUD banner from the jar's `Pause` packet is not ported (a client/UI half).
> 56 tests; scenario I proves the paused window, the manual backup while paused and the resumed
> archive, and `SELFTEST=pause` proves that assertion can fail. 51,968 B, sha256 `13fb3e0a…`.

## Slice 6 (0.2.6, 2026-11-24) — the persisted schedule counter

**Gap line.** The coverage table above: *"partial / chains / merge, regex ignores, persisted
schedule | not ported | gaps: the pin has no game-time API for tick persistence, and a regex needs a
crate"*, and *Gaps after the migration* item 4. We close **only** the persisted-schedule half;
regex ignores stays `not ported` — R9 forbids a registry dependency, so no `regex` crate. It fits
**server-only / logic**: one pure rule, one state file, one extra field on the existing `status`
line; no new hook, block entity, menu, renderer or carried content. Same shape as 0.2.5's pause
flag, which is the route this slice reuses.

**Upstream rule (re-read from `ports/_shared/upstream/simplebackups/FINDINGS.md`).**
`BackupThread.java:115-124` computes tick mode as `gameTime - lastSaved >= timer`; `lastSaved` is a
`SavedData` (`BackupData.java:11-52`) so a restart does not reset the schedule, and
`tryCreateBackup` updates it as soon as the worker **starts** (`:91-95`), not when it succeeds. Our
`start_backup` already sets `last_tick` at that moment, so the persisted value is the **progress**
`ticks.saturating_sub(last_tick)`; restoring it as `ticks = saved, last_tick = 0` makes the existing
`due_by_ticks` fire after the remaining interval.

**Port decisions.** `SCHEDULE_FILE = "simplebackups-schedule.txt"` lives beside the world (not in
the deletable backup folder), written with a staged rename on every `world_saved` checkpoint and on
graceful close/stop, and read in `ensure_world` when the world opens (the `SavedData` load moment).
`runtime.rs` gains `schedule_progress`, `format_schedule_ticks` and `parse_schedule_ticks`; only an
exact `ticks = <n>` line is accepted, so an absent or hand-edited file starts a fresh interval.
`/simplebackups status` appends `ticks=<progress>` to the existing `saves=` line (the older
substring assertions still match). The four 0.2.4 fixes and the 0.2.5 pause flag are untouched.

**Red → green.** Three pure rules and the state-file round-trip were written as failing tests first
(`sdk-test-red-ticks.log`):

```
---- runtime::tests::a_restored_counter_resumes_the_interval stdout ----
thread 'runtime::tests::a_restored_counter_resumes_the_interval' (150808) panicked at src/runtime.rs:849:9:
assertion `left == right` failed
  left: 0
 right: 12

---- runtime::tests::the_persisted_tick_counter_round_trips_through_its_file stdout ----
thread 'runtime::tests::the_persisted_tick_counter_round_trips_through_its_file' (150836) panicked at src/runtime.rs:839:9:
assertion `left == right` failed: the file is one exact line
  left: ""
 right: "ticks = 17\n"

---- support::tests::the_schedule_counter_is_persisted_beside_the_world stdout ----
thread 'support::tests::the_schedule_counter_is_persisted_beside_the_world' (150848) panicked at src/support.rs:1207:9:
assertion `left == right` failed: the counter must survive being re-read from disk
  left: 0
 right: 17

test result: FAILED. 56 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

Green: **59 passed, 0 failed** (35 `runtime`, 12 `support`, 12 `zip`) — `sdk-test.log`, clippy
`-D warnings` 0 warnings (`sdk-clippy.log`), `fmt --check` clean (`sdk-fmt.log`).

**In game.** Launcher build `native-0a875da-84cd87494e3d`, `Compiling simplebackups (1 of 1)`,
every packed source byte-identical to this tree (`../_shared/evidence/simplebackups-e2e/build-record.txt`).
Scenario J reuses one scratch home across `ctl quit` and relaunch; the counter is *seeded* to a known
`ticks = 10` after the quit snapshot is confirmed, so a nonzero restored value cannot be launch drift.
Only the newest `saves=` line is parsed (`newest_status_line` retries, so a lagging chat reply can no
longer kill the leg with a silent `EXIT:1`):

```
fresh-world backup: …/SimpleBackupsSchedule_2026-11-24_03-50-56.zip
zip entries: 20
counter before quit: 11 (saves=0 retries=0 stale=false paused=false ticks=11)
state file after quit: ticks = 17
seeded state file: ticks = 10
counter after relaunch: 28 (saves=0 retries=0 stale=false paused=false ticks=28)
resumed window: no new zip across 9 ticks (ticks=28 at relaunch)
status after the resumed window: saves=0 retries=0 stale=false paused=false ticks=43
fresh zip after resume: …/SimpleBackupsSchedule_2026-11-24_03-51-13.zip
== scenario J2: a fresh world waits the full interval ==
fresh world: saves=0 retries=0 stale=false paused=false ticks=0
fresh world: no zip across 37 ticks (ticks=0 at read)
fresh-world differential: no state file, ticks=0 at read, one zip after the full interval
PASS: … the tick counter resumes from the persisted state beside the world across quit and relaunch,
      while a fresh world with no state file waits the full interval (scenarios J, J2)
EXIT:0
```

The empty window is sized from the restored `ticks` rather than a fixed 20: the headless harness
advances the mod's counter during ctl round-trips, so a literal 20-tick window could false-red when
the restored value was already near the interval. The **parsed status** is the resume proof; the
fresh-world differential backs it. `SELFTEST=ticks` inverts that parsed value and ends
`FAIL: TICKS SELFTEST: the counter resumed (22 >= 10) — the pre-fix reset is gone, so the assertion is alive`
+ `EXIT:1`. Control `EXIT:1`, `SELFTEST=1` `EXIT:1`, `SELFTEST=pause` `EXIT:1`, `STALE_SELFTEST=1`
`EXIT:1`, the two older negatives `EXIT:1`; `check-e2e-logs.py simplebackups-e2e --strict` →
`logs=8 ok=8 missing-exit=0 bad-exit=0`. All seven earlier legs stayed green on the 0.2.6 game.

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices

### Package 0.2.6

| | |
|---|---|
| version | **0.2.6** (`horizon-mod.toml` + `Cargo.toml`) |
| bytes | **53,344 B** |
| sha256 | `a2261c76ce1f2e6df592262279df92573a9cecceb687f1624f1f44d260294c09` |
| entries | **10** — `horizon-mod.toml`, `Cargo.toml`, `LICENSE`, `LICENSE-NOTICE.md`, `README.md`, `src/{lib,runtime,support,zip}.rs`, `uses.json`; **no `Cargo.lock`** |
| reproducible | two builds, each after `rm -f Cargo.lock`: identical sha and `cmp` |
| `check` | `simplebackups 0.2.6 (native-mod, format native-1/0a875da…/e59da90…, for 26.3)`, 53,344 bytes, exit 0 |
| `fits --build 0a875da91dbb0fdcb7ba8c42c11341aa229af51a` | `0a875da  made for` (the pin tautology) |
| `fits --build 0b14f8acfd97fa2c750e42795249dcc981178ca4` | `0b14f8a  fits` — the real gate |
| tests | **59 passed, 0 failed** (35 `runtime`, 12 `support`, 12 `zip`) |
| `verify-sdk-package.py simplebackups` | `1/1`, `package matches the tree` |

**Remote build path.** Every cargo ladder step forwarded through
`ports/tools/build-slot.sh run --job W1-simplebackups --port ports/simplebackups --step …` to host
**AdLf-PC** (`build-slot: forwarding … to the remote runner`); `BUILD_LOCAL=1` was never set. The
launcher native build and all eight game legs stayed local: the second box has no
`horizon-launcher` or Nightly versions cache (checked over ssh), so a forwarded game leg cannot
build or launch. The launcher build ran inside a slot lease with `FORWARDED=1` (keep-local marker,
not `BUILD_LOCAL`), producing the `Compiling simplebackups (1 of 1)` line above. All e2e scratch
dirs came from `E2E_TMP` under `~/.cache/horizon-e2e/scratch` — never `/tmp`.

### Changelog (0.2.6)

> **0.2.6 — the tick schedule survives a restart.** The progress toward the next scheduled backup
> is stored in `simplebackups-schedule.txt` beside the world with a staged rename, written on each
> save and on a graceful close, and restored when the world opens — upstream's `lastSaved` in its
> `BackupData` SavedData, which a restart does not reset. `/simplebackups status` reports
> `ticks=<progress>`, so a parsed reply after relaunch proves the counter resumed. Regex ignores
> remain out of scope (R9). 59 tests; scenario J (quit and relaunch the same world) and scenario J2
> (a fresh world's full interval) prove it in game.

## Slice 7 (0.2.7, 2026-11-24) — incremental chains (base + changed-only children, merge)

**Gap line.** The coverage table above: *"partial / chains / merge, regex ignores | not ported |
gaps: incremental/chains/merge is a larger design task; a regex needs a crate (R9 forbids a
registry dependency)"*, and *Gaps after the migration* item 4. We close the **partial / chains /
merge** half; regex ignores stay `not ported` (R9 forbids the crate). It fits **server-only /
logic**: pure rules in `runtime.rs`, the existing worker/publish path, one persisted state file and
one extra command — no new hook, block entity, menu, renderer or dependency. `horizon/docs/modpacks.md`
puts "backups" in the server-only class and "server events, typed commands" in the logic tier.

**Upstream format (read from the re-cloned `1.21.1` head `bf04de3e7b3e05504ee5cc974cf45833b0d4b9a4`,
`ports/_shared/upstream/simplebackups-src/`).** `BackupChain.java`, `BackupChainManager.java`,
`ExperimentalConfig.java`, `BackupThread.java`, `commands/MergeCommand.java`, `BackupData.java`:

- `metadata.json` (compact Gson, keys in this order) holds `backupType` (`FULL_BACKUPS` /
  `INCREMENTAL` / `DIFFERENTIAL`), `fullBackup` (`"full.zip"`), `children`
  (`"child-0001.zip"`, `child-%04d` with index = children.size()+1) and `lastUpdated` (millis).
- **Base choice** (`BackupThread.java:288-301`): a new full chain (`createChain`, dir named for the
  UTC stamp) when `fullBackup` — no data, FULL_BACKUPS, or `now - fullBackupTimer > lastFullBackup`
  (default `525960` min) — or when no chain exists; otherwise `latestChain.createChild()`.
- **Changed entries** (`:394-395`): `lastModified - lastSaved > 0` (strict); `lastSaved` is
  `BackupData.lastSaved` for `MODIFIED_SINCE_LAST` (incremental) and `lastFullBackup` for
  `MODIFIED_SINCE_FULL` (differential). A full base carries every file. `lastSaved` is updated when
  the worker *starts*, not when it succeeds (`:91-95`).
- **Retention** in experimental mode deletes whole chain dirs, oldest by `lastUpdated`, down to
  `backupsToKeep`, never the last; the cap deletes oldest chains but keeps one.
- **Merge** (`MergeCommand.java:70-81`): INCREMENTAL = full + all children, DIFFERENTIAL = full +
  last child; per entry name the newest archive wins; output `merged_backup-<UUID>.zip`.

**Port decisions.** The port keeps its flat `<world>_<UTC>.zip` archives and the 0.2.3/0.2.4
retention identity, so the chain facts travel in each archive's own `simplebackups.json` instead of
a directory's `metadata.json`. The manifest **keeps `format = simplebackups/1`** (retention is
untouched) and gains `chain` (base name), `kind` (`full`/`child`), `changed` (the entries this
archive holds) and `len` (chain length). `ChainState` (`base`, `children`, `len`, `last_backup_ms`,
`last_full_ms`) is persisted in `simplebackups-chain.txt` beside the world with a staged rename and
restored when the world opens — upstream's `BackupData` SavedData, which a native mod lacks. Pure
rules: `parse_backup_type`, `full_timer_due`, `choose_kind`, `changed_threshold`, `entry_changed`,
`merge_order`, the chain-state file round-trip and the manifest's parser/writer. Config gains
`backup_type` (default `"full"`, so every earlier scenario and package behaviour is unchanged) and
`full_backup_timer` (minutes, default 525960). `/simplebackups status` appends `chain=<n>` to the
existing `saves=` line; `/simplebackups merge` folds the chain into `merged_backup-<UTC>.zip`.
The port's own `simplebackups-*.txt` state files are skipped by the walk, so they cannot appear as
"changed" entries. The 0.2.4 adoption/retention fixes, the 0.2.5 pause flag and the 0.2.6 schedule
counter are byte-for-byte unchanged in behaviour.

**Red → green.** Nine failing tests first, against stubs (`sdk-test-red-chain.log`):

```
---- runtime::tests::a_changed_entry_is_one_modified_after_the_threshold stdout ----
thread 'runtime::tests::a_changed_entry_is_one_modified_after_the_threshold' (333845) panicked at src/runtime.rs:1487:9:
a file written after the previous backup is changed

---- runtime::tests::backup_types_parse_from_both_the_config_and_upstream_metadata stdout ----
assertion `left == right` failed
  left: None
 right: Some(Full)

---- runtime::tests::the_first_backup_of_a_chain_is_full_and_later_ones_are_children stdout ----
assertion `left == right` failed
  left: Full
 right: Child

---- runtime::tests::the_chain_manifest_names_the_base_kind_changed_entries_and_length stdout ----
assertion `left == right` failed
  left: None
 right: Some("World_2026-01-01_00-00-00.zip")

---- support::tests::the_chain_state_is_persisted_beside_the_world stdout ----
assertion `left == right` failed: the base and the changed-entry thresholds must survive being re-read from disk
  left: ChainState { base: None, children: [], len: 0, last_backup_ms: 0, last_full_ms: 0 }
 right: ChainState { base: Some(...), children: [...], len: 2, last_backup_ms: 9000, last_full_ms: 1000 }

test result: FAILED. 59 passed; 9 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
```

Green: **68 passed, 0 failed** (39 `runtime`, 12 `support`, 17 `zip`) — `sdk-test.log`; clippy
`-D warnings` 0 warnings (`sdk-clippy.log`); `fmt --check` clean (`sdk-fmt.log`).

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices

### Package 0.2.7

| | |
|---|---|
| version | **0.2.7** (`horizon-mod.toml` + `Cargo.toml`) |
| bytes | **60,637 B** |
| sha256 | `f02f1aa1786e5b0d141ec4bed2e94535498a60c4bc600e4dc1a2a4af595f9656` |
| entries | **10** — `horizon-mod.toml`, `Cargo.toml`, `LICENSE`, `LICENSE-NOTICE.md`, `README.md`, `src/{lib,runtime,support,zip}.rs`, `uses.json`; **no `Cargo.lock`** |
| reproducible | two builds, each after `rm -f Cargo.lock`: `cmp` identical, same sha |
| `check` | `simplebackups 0.2.7 (native-mod, format native-1/0a875da…/e59da90…, for 26.3)`, 60,637 B, exit 0 |
| `fits --build 0a875da91dbb0fdcb7ba8c42c11341aa229af51a` | `0a875da  made for` |
| `fits --build 0b14f8acfd97fa2c750e42795249dcc981178ca4` | `0b14f8a  fits` — the real gate |
| tests | **68 passed, 0 failed** |
| `verify-sdk-package.py simplebackups` | `1/1`, `package matches the tree`, R9 clean |

**In game (launcher build `native-0a875da-d16da12df046`, `Compiling simplebackups (1 of 1)`;
every packed source byte-identical to this tree — `../_shared/evidence/simplebackups-e2e/build-record.txt`).**
Scenario K's transcript (`e2e-launcher.log`):

```
full base: …/SimpleBackupsChain_2026-11-24_06-46-22.zip
base manifest: kind=full chain=SimpleBackupsChain_2026-11-24_06-46-22.zip len=1
child backup: …/SimpleBackupsChain_2026-11-24_06-46-22-1.zip
child manifest: chain=SimpleBackupsChain_2026-11-24_06-46-22.zip kind=child len=2 changed=SimpleBackupsChain/changed-marker.txt
the child carries only the changed entry; the base and the change are in the manifest
status: saves=0 retries=0 stale=false paused=false ticks=27 chain=2
list: SimpleBackupsChain_2026-11-24_06-46-22-1.zip (579 B)
      SimpleBackupsChain_2026-11-24_06-46-22.zip (4.1 KB)
merged archive: 8 entries, kind=full
merge folded the chain into one standalone archive
fresh-world first backup: …/SimpleBackupsChainFresh_2026-11-24_06-46-35.zip
fresh-world manifest: kind=full chain=SimpleBackupsChainFresh_2026-11-24_06-46-35.zip len=1
fresh-world status: saves=0 retries=0 stale=false paused=false ticks=6 chain=1
fresh-world differential: the first backup is full with chain=1
PASS: … scenario K …
EXIT:0
```

The scene is fail-capable: `SELFTEST=chain` (`e2e-launcher-chain-selftest.log`) inverts the base
assertion and ends `FAIL: CHAIN SELFTEST: the child manifest named the base (…) — the assertion is
alive` + `EXIT:1`. To keep "only the changed entry" deterministic in a live game, the chain config
sets `ignored_paths = ["dimensions", "region"]`: the game writes region/POI files between the two
backups, and a world file outside those volatile subtrees (`changed-marker.txt`) is the one the
child carries. Every earlier leg stayed green on the 0.2.7 game: `PASS` + `EXIT:0` (scenarios
A–K); control `EXIT:1`; `SELFTEST=1`, `SELFTEST=pause`, `SELFTEST=ticks`, `SELFTEST=chain`,
`STALE_SELFTEST=1`, `ADOPT_NEGATIVE=1`, `SAME_SECOND_NEGATIVE=1` all `EXIT:1`.
`check-e2e-logs.py simplebackups-e2e --strict` → `logs=10 ok=10 missing-exit=0 bad-exit=0`.
Two harness flakes were hardened in `tools/e2e.sh` (no code change): scenario I's unguarded
`grep -m1 'saves='` now uses the retrying `newest_status_line`, and scenario J's counter sample
retries until it lands mid-interval (a busy box can advance the headless counter a full interval
during one zip check).

### Changelog (0.2.7)

> **0.2.7 — incremental chains and merge.** With `backup_type = "incremental"` the first backup
> becomes a full base and each later one carries only files modified since the previous backup;
> `"differential"` compares against the last full. Each archive's `simplebackups.json` keeps the
> `simplebackups/1` identity and gains the base name, the kind, the changed entries and the chain
> length (upstream's `metadata.json` facts, kept in the archive so the flat retention is untouched).
> The chain state persists beside the world like the pause flag and the schedule counter, restoring
> across restarts (upstream's `BackupData`). `status` reports `chain=<n>`; `/simplebackups merge`
> folds a chain into one standalone archive. The default `backup_type` is `"full"`, so existing
> behaviour is unchanged. Regex ignores stay out of scope (R9). 68 tests; scenario K proves the
> base/changed-entry rule, `chain=2`, `list`, merge and the fresh-world `chain=1` differential in
> game. 60,637 B, sha256 `f02f1aa1…`.

## Slice 8 (0.2.8, 2026-11-24) — the empty-server policy

**Gap line.** The coverage table above: *"empty-server policy (`noPlayerBackups`) | not ported |
gap; needs a player-presence API and a way to empty the server headless"*, and *Gaps after the
migration* item 4. The first half is now available: at pin `0a875da` the SDK's players service
(`host.players().server_players()` → `native_server_players()`) reports the server player list.
The second half is **not** available through `horizon-ctl` (see the harness limit below), so the
empty branch is proven unit-first rather than faked in game. Class **server-only**, tier **logic**
(ledger authority). Shape: two config keys, one pure gate, one persisted counter beside the world,
two extra fields on the existing `saves=` status line. No new hook, block entity, menu, renderer,
carried content or dependency. Regex ignores stay `not ported` (R9 forbids a registry dependency).

**Upstream policy (re-read from the re-cloned `1.21.1` head `bf04de3e7b3e05504ee5cc974cf45833b0d4b9a4`,
`ports/_shared/upstream/simplebackups-src`, deleted after these findings were recorded).**
`EventListener.java:43` defines "no player online" as the **server player list being empty**
(`!server.getPlayerList().getPlayers().isEmpty()` is false), not a timeout since the last input and
not per shutdown: the counter is **per empty period**, reset on a join. `:44-50`: with nobody
online the tick proceeds only when `noPlayerBackups` is set **and** `noPlayerBackupCount != 0` **and**
`backupsSinceLastPlayerJoined < noPlayerBackupCount`. With a player online the gate is bypassed.
`:52-57`: the counter increments when the backup is **started** (not when it finishes), and only
for an empty-server scheduled backup. `:64-66`: `PlayerLoggedInEvent` resets the counter;
`:74-81` re-enables the gate when the last player leaves. `BackupData.java:18,32,50,81-97` stores
the counter in the `simplebackups` SavedData, so it survives a restart. `CommonConfig.java:86-89`:
`noPlayerBackups` defaults to `false`, `noPlayerBackupCount` to `1`, and `0` disables empty-server
backups. `BackupThread.java:103-107,115-123` keeps `shouldRunBackup`'s enabled/paused/due gate, so
the presence rule **wraps** it rather than replacing it. Upstream's separate `noPlayerBackupTimer`
(`:90-91,151-157`, default 0 → use `timer`) is out of this brief's scope: the empty period uses the
same `interval_ticks`/`interval_minutes` and this deviation is recorded.

**Port decisions.** `runtime.rs` gains `scheduled_backup_permitted(enabled, paused, online,
no_player_backups, count, since)` (the presence rule wrapping `schedule_allowed`), `format_no_player_count`
/ `parse_no_player_count`, and the two `Settings` keys with their defaults and round-trip. `Core`
gains `players`, `seen_players` and `empty_backups`; `NO_PLAYER_FILE = "simplebackups-no-player.txt"`
lives beside the world (not in the deletable backup folder), is read in `ensure_world` (the SavedData
load moment) and written with a staged rename on each change/save/close. With no new hook available,
upstream's login reset is detected on the **empty→player transition** sampled at each tick (including
the owner's join as the world opens). `start_backup` increments the counter only for a scheduled
backup started while empty, exactly at upstream's `tryCreateBackup` moment. `/simplebackups status`
appends `players=<n> empty_backups=<n>` to the existing `saves=` line; earlier substring assertions
stay valid. A manual backup ignores the presence rule, as upstream's `createBackup` ignores
`shouldRunBackup`. The seven earlier slices' behaviour is unchanged.

**Harness limit (measured, not assumed).** `horizon-ctl launch` opens a single-player integrated
world whose owner connects through the local hub, so `host.players().server_players()` is non-empty
for the entire run. `ctl disconnect` refuses outside a remote server, and there is no dedicated
launch in the harness. Scenario L's first `status` measured `players=1 empty_backups=0` on both the
0.2.8 game (`e2e-launcher.log`) and the diagnostic (`e2e-launcher-noplayer-diag.log`; the game's own
`state` reply names the joined owner). A genuinely playerless interval therefore **cannot** be
produced here, exactly the fallback the brief allows: no chat command is dressed up as an empty
server. The empty branch, the `0` disables rule and the `count` cap are proven red-first in pure
tests; the in-game leg asserts the online branch and reports the observed player count.

**Red → green.** Five failing tests first, against stubs (`sdk-test-red-noplayer.log`; verbatim):

```
---- runtime::tests::an_empty_server_never_schedules_by_default stdout ----
thread 'runtime::tests::an_empty_server_never_schedules_by_default' (563117) panicked at src/runtime.rs:1877:9:
assertion failed: !scheduled_backup_permitted(true, false, false, false, 1, 0)

---- runtime::tests::no_player_count_caps_backups_per_empty_period stdout ----
thread 'runtime::tests::no_player_count_caps_backups_per_empty_period' (563131) panicked at src/runtime.rs:1886:9:
assertion failed: !scheduled_backup_permitted(true, false, false, true, 0, 0)

---- runtime::tests::no_player_keys_round_trip_through_the_config stdout ----
thread 'runtime::tests::no_player_keys_round_trip_through_the_config' (563132) panicked at src/runtime.rs:1909:9:
["unknown key: no_player_backups", "unknown key: no_player_backup_count"]

---- runtime::tests::the_empty_backup_counter_round_trips_through_its_file stdout ----
thread 'runtime::tests::the_empty_backup_counter_round_trips_through_its_file' (563150) panicked at src/runtime.rs:1899:9:
assertion `left == right` failed
  left: ""
 right: "backups = 3\n"

---- support::tests::the_no_player_counter_is_persisted_beside_the_world stdout ----
thread 'support::tests::the_no_player_counter_is_persisted_beside_the_world' (563167) panicked at src/support.rs:2060:9:
assertion `left == right` failed: the counter must survive being re-read from disk
  left: 0
 right: 4

test result: FAILED. 68 passed; 5 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

Green: **73 passed, 0 failed** — `sdk-test.log`, clippy `-D warnings` 0 warnings (`sdk-clippy.log`),
`fmt --check` clean (`sdk-fmt.log`). Clippy caught one redundant boolean in the join detection; it
was rewritten as a named `player_joined` condition rather than allowed.

**In game.** Launcher build `native-0a875da-9f57a9d12e7e`, `Compiling simplebackups (1 of 1)`,
every packed source byte-identical to this tree (`../_shared/evidence/simplebackups-e2e/build-record.txt`).
Scenario L's transcript (`e2e-launcher.log`):

```
no-player status (before): saves=0 retries=0 stale=false paused=false ticks=20 chain=0 players=1 empty_backups=0
harness: the owner is online (players=1); a genuinely playerless interval is not producible by ctl
online-window assertion: a scheduled backup appeared with players=1
zip entries: 13
no-player status (after): saves=0 retries=0 stale=false paused=false ticks=9 chain=1 players=1 empty_backups=0
```

Scenario L2 repeats with `no_player_backups = true` and `no_player_backup_count = 1` (`players=1
empty_backups=0` before and after, a fresh zip at `interval_ticks=40`); the owner is again online
and the schedule still produces a zip. `SELFTEST=noplayer` (`e2e-launcher-noplayer-selftest.log`)
inverts the online-window assertion — `FAIL: NOPLAYER SELFTEST: a scheduled backup appeared with a
player online — the online-window assertion is alive (no empty window was available to invert)` +
`EXIT:1` — and is explicitly **not** presented as the empty-window inversion, which the harness
cannot run. All ten earlier scenario legs stayed green on the 0.2.8 game; control `EXIT:1`,
`SELFTEST=1`, `SELFTEST=pause`, `SELFTEST=ticks`, `SELFTEST=chain`, `SELFTEST=noplayer`,
`STALE_SELFTEST=1`, `ADOPT_NEGATIVE=1`, `SAME_SECOND_NEGATIVE=1` all `EXIT:1`.
`check-e2e-logs.py simplebackups-e2e --strict` → `logs=11 ok=11 missing-exit=0 bad-exit=0`.

### Package 0.2.8

| | |
|---|---|
| version | **0.2.8** (`horizon-mod.toml` + `Cargo.toml`) |
| bytes | **63,311 B** |
| sha256 | `843d9fd8ba3aba26ba0f03e3fe9b295dc30efa182afa75e19466e94a39ebb117` |
| entries | **10** — `horizon-mod.toml`, `Cargo.toml`, `LICENSE`, `LICENSE-NOTICE.md`, `README.md`, `src/{lib,runtime,support,zip}.rs`, `uses.json`; **no `Cargo.lock`** |
| reproducible | two builds, each after `rm -f Cargo.lock`: identical sha and `cmp` |
| `check` | `simplebackups 0.2.8 (native-mod, format native-1/0a875da…/e59da90…, for 26.3)`, 63,311 B, exit 0 |
| `fits --build 0a875da91dbb0fdcb7ba8c42c11341aa229af51a` | `0a875da  made for` (the pin tautology) |
| `fits --build 0b14f8acfd97fa2c750e42795249dcc981178ca4` | `0b14f8a  fits` — the real gate |
| tests | **73 passed, 0 failed** (39 `runtime` → 43, 12 `support` → 13, 12 `zip`) |
| `verify-sdk-package.py simplebackups` | `1/1`, `package matches the tree`, R9 clean |

**Remote build path.** Every cargo ladder step forwarded through
`ports/tools/build-slot.sh run --job W1-simplebackups --port ports/simplebackups --step …` to host
**AdLf-PC** (`build-slot: forwarding … to the remote runner`); `BUILD_LOCAL=1` was never set. The
launcher native build and every game leg stayed local (`FORWARDED=1`), because the second box has no
`horizon-launcher` or Nightly versions cache. All e2e scratch dirs came from `E2E_TMP` under
`~/.cache/horizon-e2e/scratch` — never `/tmp`.

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices

### Changelog (0.2.8)

> **0.2.8 — the empty-server policy.** With `noPlayerBackups=false` (upstream's default) the
> schedule is silent while the server player list is empty; opting in with `noPlayerBackups=true`
> allows at most `noPlayerBackupCount` backups (default 1, 0 disables) per empty period until a
> player joins. The counter is incremented when an empty-server backup **starts** and resets when a
> player joins, and is persisted in `simplebackups-no-player.txt` beside the world (upstream's
> `backupsSinceLastPlayerJoined` SavedData). `/simplebackups status` reports `players=<n>
> empty_backups=<n>`. A manual backup ignores the presence rule. Upstream's separate
> `noPlayerBackupTimer` is not ported. Regex ignores stay out of scope (R9). 73 tests; scenario L
> proves the online branch with the observed player count, and the empty branch is proven red-first
> by pure tests because a `ctl` launch always has the owner online. 63,311 B, sha256 `843d9fd8…`.

## Verification (re-pin `83ad384f`, 2026-11-06/07)



Staged at Horizon `83ad384f48d0742746d518737511882680aab5b7` / horizon-mixins
`e8cd0aa38cffef49cb2e088ef716baa107148c02` with `CARGO_INCREMENTAL=0`.

| Check | Evidence |
|---|---|
| runtime tests (`horizon.py test`, embedded) | `test result: ok. 16 passed; 0 failed; 0 ignored; 913 filtered out`; **0 warnings** (`target/horizon-test.log`) |
| `horizon.py check` | `cargo_exit: 0`; `target/horizon-check.log` has **0** `warning:` lines |
| `horizon.py build` | `cargo_exit: 0`; `target/horizon-cargo/fast/horizon-client` (96,163,344 B, 23:25) |
| `horizon-ctl` build | exit 0; `target/horizon-cargo/fast/horizon-ctl` |
| `tools/e2e.sh` | exit 0, `PASS` (below); log `target/G9-e2e.log` |

The overlap scenario forces a save while the 64 MiB payload is being zipped
(the integrated server saves on the pause tick, so `world_saved` fires inside
the copy) and asserts the retry counter. `ctl pause` is used rather than
`/tick sprint`: the world's first autosave is 6000 ticks away and a sprint only
reaches a few hundred ticks per second on this box, so it cannot land inside a
~1 s copy.

```
status: saves=3 retries=1 stale=false ... last=SimpleBackupsOverlap_....zip
status: saves=2 retries=0 stale=true  ... last=SimpleBackupsOverlapOff_....zip
PASS: scheduled zip valid, keep=1 pruned, disabled schedule silent, manual works
while disabled, an overlapping save is retried (and marked stale when retry is off)
```

The second run is the differential: with `retry_on_save = false` the same
overlap is published and reported `retries=0 stale=true`, so an implementation
that ignored the save would fail the first assertion.

## Verification results (in-game)

`tools/e2e.sh`, two headless worlds, exit **0**:

| Phase | Evidence |
|---|---|
| scheduled (`interval_ticks=40`) | `SimpleBackupsE2E_2026-11-03_10-24-58.zip`, 27 entries, contains `level.dat`, no `session.lock`/`backups/` |
| manual `/simplebackups backup` | a newer zip (`…10-24-59.zip`, 30 entries); `keep=1` left exactly one |
| `enabled=false` | 100 ticks with no zip |
| manual while disabled | one zip (28 entries) |

`PASS: scheduled zip valid, keep=1 pruned, disabled schedule silent, manual
works while disabled`.

## Gaps / deviations

- **Consistency window**: there is no save-off/autosave-veto API at the pin; the
  worker can overlap an autosave and copy a region file mid-replacement (the
  upstream mod has the same race; `save_game()` flushes, which is better than
  upstream). A synchronous copy or a future save-off hook would close it.
- The schedule counter is now persisted across restarts (slice 6): the progress toward the next
  backup is written beside the world and restored when it opens, so a restart resumes the interval
  instead of starting at zero. Only tick-mode progress is stored; a wall-clock interval still starts
  at world open (the game's own wall clock is not available through the SDK). The empty-server
  policy is now ported (slice 8) — a `ctl` launch always has the world owner placed, so its
  playerless branch is proven by unit tests, not in game; upstream's separate `noPlayerBackupTimer`
  is now ported too (slice 12): an empty period uses that many minutes, 0 reuses the main interval,
  and its two branches are proven red-first by unit tests (the harness still cannot be playerless).
  `latest.log` capture is now ported (slice 9); Cherished
  Worlds/mc2discord and regex ignores stay deferred. The disk cap
  (slice 2), `pause`/`unpause` (slice 5), the persisted counter (slice 6) and incremental
  chains/merge (slice 7) are now ported. Chain children are not standalone restore points; the flat
  retention (slice 3/4) still prunes by archive identity and is not chain-aware — deliberately
  unchanged, so an operator can keep a full base by setting `keep` above the chain length.
- Retention prunes after success (upstream deletes before creating), only archives the port can
  identify as its own (a manifest since 0.2.3; a marker-less 0.2.0–0.2.2 archive only after an
  operator's `/simplebackups adopt <file>`), and after a stale copy it prunes nothing at all; all
  are deliberate safety changes.
- Naming is UTC; upstream uses local time. Backup location is
  `<world>/backups` (upstream `<cwd>/simplebackups/<world>`).
- `native_world_dir` is a pin gap: it is not cleared after a close, so the port
  trusts it only between `WorldOpenReady` and close.
- Licence: upstream is **Apache-2.0** while the template `Cargo.toml`/LICENSE
  are MIT for the port code; review/attach the Apache-2.0 notice before any
  publication (local-only now).

## Upstream source

- **Repo:** <https://github.com/ChaoticTrials/SimpleBackups> (Apache-2.0).
- **Pack version:** `1.21-4.0.30` (Modrinth `fzSKSXVK`, published
  2026-04-22T09:23:33Z). No `4.0.x` tag exists; the matching source is branch
  **`1.21.1`**, head **`bf04de3e7b3e05504ee5cc974cf45833b0d4b9a4`**
  (2026-04-22 09:20 UTC; `build.gradle` `modBaseVersion = '1.21-4.0'`).
  `main` is a later 26.3 rewrite with a different `sbk` format and is **not**
  the pack's mod.
- **Read:** the full `1.21.1` source tree — `SimpleBackups`, `EventListener`,
  `BackupThread`, `BackupData`, `BackupChain*`, `StorageSize`, `commands/*`,
  `config/*`, `compat/*`, `network/Pause`, `lang/en_us.json`, `build.gradle`.
  Full evidence with `file:line` in
  `ports/_shared/upstream/simplebackups/FINDINGS.md`.
- **Behaviour extracted:** full zip backups of `world/` on a 120-minute
  wall-clock default (or game-tick mode), persisted `lastSaved`, only while
  players are online unless configured; `saveEverything(true,false,true)`
  before the copy (no flush); zip entries `<levelId>/<relative path>`,
  deflate level −1, `session.lock` skipped; retention deletes before create
  by mtime with `backupsToKeep=10`, plus a 25 GB cap; commands
  `/simplebackups backup start [quiet]`, `backup pause|unpause`,
  `mergeBackups`, level 2; the zip runs on a `Thread` while the server keeps
  ticking.
- **Port decisions:** `save_game()` (synchronous, flushes chunks and writes
  `level.dat`) instead of the console `save-all` (the pin's in-game dispatcher
  has no `save-all`). Backups go to `<world>/backups/` and are excluded from
  the zip; prune only `<world>_*.zip` and only after a successful rename (a
  failed backup never deletes the last good one). Schedule with our own
  `ServerTickEnd` counter (`NativeTickId` has no public accessor), optional
  wall-clock minutes, explicit `interval_ticks` for tests. Manual command
  queues the save+spawn for `ServerTickEnd` (no `save_game` inside a player
  hook). Config `config/simplebackups.toml`; commands
  `simplebackups backup [quiet]`, `status`, `list`, level 2.
- **Save overlap (re-pin slice):** there is no save-off/autosave-veto API for
  native mods, so the off-thread zip can overlap the next autosave (every 6000
  ticks, `game.rs:6840-6845`). The port now detects it: `world_saved` (API 306)
  bumps a save sequence, the in-flight copy records the sequence when it starts,
  and a finished copy whose `save_seq` moved is **thrown away and redone**
  (`retry_on_save = true`, up to 2 retries) instead of being published. When
  retries are off or exhausted the copy is published and reported as stale
  (`/simplebackups status` shows `retries=` and `stale=`). The e2e forces the
  overlap with `/tick sprint 3` (the world's first autosave is 6000 ticks
  away) over a 64 MiB payload; the differential is `retry_on_save = false`,
  which must publish the copy with `retries=0 stale=true`. The 26.3 supervisor
  (`crates/horizon-server/src/api.rs:468-494`) instead uses `save-off` →
  `save-all flush` → zip → `save-on`; a native mod has no such veto yet, so a
  retried copy is the faithful approximation.
- **Pin caveat:** `native_world_dir()` reads `world_root`, which is set in
  `finish_open_world` (`game.rs:11495`) but never cleared on close, so it is
  **None** before the first open and a same-instance `Game` can keep a stale
  path after its world closed. Trust it only between `WorldOpenReady` and
  `WorldCloseFinal`; do not read it in `server_about_to_start`.
  `native_config_dir()` is static and safe anywhere.
- **Deferred (as of the 2026-11-03 analysis; pause and the disk cap were later ported — slices 5
  and 2; the persisted schedule slice 6, chains/merge slice 7, empty-server policy slice 8):**
  Cherished Worlds, mc2discord, regex ignores, sync-copy mode, and upstream's
  separate `noPlayerBackupTimer`. (`latest.log` capture was later ported — slice 9; the empty-server
  interval was later ported — slice 12; sync-copy stays `blocked(no-sync-observable)` — slice 11.)
- **Licence:** Apache-2.0 permits derivatives, but the port stays local-only
  until the maintainer approves publishing (ARCHITECTURE.md §8). The scaffold's
  `Cargo.toml` currently declares `MIT` while upstream is Apache-2.0; the
  rewrite is clean-room, but the outbound licence/notice needs review before
  any publication (no action while local-only).

## Migration to the SDK format (0.2.0, 2026-11-18)

**Why.** Horizon's mod-API merge (#16941, launcher ≥ 0.2.70) retired `.hnmod`; the registry closed
every `.hnmod` submission with one reason. The unit is now **one flat `.hmod`** — `horizon-mod.toml`
(`api = "0.2"`, exact `[native]` pins), `Cargo.toml`, `src/lib.rs` with one
`#[mod_api::horizon_mod] impl Hooks`, and `src/runtime.rs` carried byte-identical. This is a **first
publication** of the mod (no earlier PR exists), version **0.2.0**.

Pins for this migration: SDK commit **`0b14f8acfd97fa2c750e42795249dcc981178ca4`**, mixins
**`e59da9094f0c2fdda6c627fe9eff22dec6a906f0`** (the mixins pin moved: `e8cd0aa3` → `e59da909`). The
port's own `runtime.rs` is unchanged, so every rule (schedule arithmetic, UTC names, retention,
skip rules, staleness) behaves exactly as the 0.1.0 bundle did.

### What moved, and where the retired tree is

Moved **before the first build** (a package root may hold package paths only), together with the
`.hnmod` format's plan and tools, into
[`../_shared/evidence/simplebackups/retired-hnmod-0.1.0/`](../_shared/evidence/simplebackups/retired-hnmod-0.1.0/):

| Moved | What it was |
|---|---|
| `mixins/simplebackups/` | `horizon-mixin.toml` (2 modules, 7 injections), `runtime.rs`, `support.rs`, 7 per-hook helpers |
| `compatibility.json` | the old `horizon_revision` + `game.rs` hash; `[native]` in the manifest is the pin now |
| `rust-toolchain.toml` | the pinned Horizon commit decides the toolchain now |
| `tools/{audit,horizon.py,package.py,test_stage.py,test_template.py}`, `tools/__pycache__` | staging, `.hnmod` packaging and the mixin-plan tests |
| `.github/`, `AGENTS.md` | the retired repo's CI and working note |
| `Cargo.lock` | deleted, not kept: a packed lock changes the artifact (trap 5 below) |

**Kept in the port:** `tools/e2e.sh` (the scenario and its assertions are format-independent — it
now names a launcher-built game), `notes.md`, `README.md` (rewritten for the package),
`VALIDATION.md`, `REPIN-AUDIT.md`, `LICENSE`, and the new `LICENSE-NOTICE.md`.

### File map (0.2.0)

| Path | What it is |
|---|---|
| `horizon-mod.toml` | the only manifest and the only identity: `id = "simplebackups"`, `version = "0.2.0"`, `api = "0.2"`, `side = "both"`, `capabilities = ["registries"]`, `[native]` pinned to `0b14f8a` / `e59da90` for 26.3 |
| `Cargo.toml` | one crate; the only dependency is `horizon-mod-api` from `.horizon/sdk/` |
| `src/lib.rs` | the one `impl Hooks`: 7 moments, each one call into `support` |
| `src/runtime.rs` | the pure policy **copied byte-for-byte** — sha256 `34d0d72063d8d0ba87de05f8f0250d166762ad182881c2ab5ad6525ff8812739`, identical before and after `unzip -p simplebackups.hmod src/runtime.rs` |
| `src/support.rs` | the glue: config, world folder, save request, worker, publish/prune, commands |
| `src/zip.rs` | **new**: a stored-entry ZIP writer (local headers, central directory, ZIP64) with nothing but `std` — the `zip` crate the bundle used is not available to a mod |
| `tools/e2e.sh` | the in-game scenario, updated for a launcher-built game (`GAME=`, optional `CONTROL_GAME=`, `SELFTEST=1`) |
| `README.md`, `LICENSE`, `LICENSE-NOTICE.md` | inside the package (the packer takes them; `notes.md`/`VALIDATION.md` are not packed) |
| `simplebackups.hmod` | the delivered package, 29,207 B, sha256 `1d565c1d1f2b90d8ceb13b31a7715339601196f3bbacb405a67f95564a1d4f38` |

No `assets/` or `data/`: the jar carries no blocks, items, recipes, tags or models, and a native
package with neither is valid. No `icon.png` either (optional; the port has none).

### Helper → hook table (every 0.1.0 injection and module, and where it went)

| 0.1.0 | 0.2.0 | Mapping |
|---|---|---|
| `[[module]] runtime` (`runtime.rs`) | `pub mod runtime` | copied byte-identical; `pub` so its items are not `dead_code` |
| `[[module]] state` (`support.rs`) | `pub mod support` | same code, no `//!`/`#![…]` requirement any more (it is an ordinary module) |
| `server_about_to_start` (`about.rs`) | `Hooks::server_about_to_start` | `support::load_config(&*host)` reads `config/simplebackups.toml` through `ConfigHost::config_files` (`Game::native_config_dir` + `fs` before) |
| `world_lifecycle` (`world.rs`) | `Hooks::world_lifecycle` | holds only the close/failure phases: **this pin has no `WorldOpenReady`** (trap 2), so the open world is taken lazily by `world_dir()` at the first running tick |
| `server_tick` (`tick.rs`) | `Hooks::server_tick` (`ServerTickPhase::ServerTickEnd { world_running }`) | same poll/schedule body, plus the lazy world and the save-timeout branch |
| `world_saved` (`saved.rs`) | `Hooks::world_saved` | counts the save sequence as before, and now also starts the copy that was waiting for the save |
| `server_stopping` (`stopping.rs`) | `Hooks::server_stopping` | identical (`closing = true`) |
| `register_commands` (`register.rs`) | `Hooks::register_commands` + `event.commands` | `NativeCommand::op(usage, 2)` → `Command::op(usage, 2)` |
| `perform_command` (`perform.rs`) | `Hooks::run_mod_command` → `Flow::Cancel` | typed: `event.command.literals` + `event.command.argument("quiet")` (`CommandArgumentValue::Boolean`) instead of splitting a raw line; feedback through `Commands::feedback` |
| `Game::save_game()` (synchronous) | `LevelRef::request_save(host)` per open level, then the copy on `world_saved` | there is no synchronous save in the SDK; the save runs at the next tick's start (trap 3) |
| `game.native_world_dir()` | `ConfigServerHost::world_dir()` | `Some` only while a world is open — the 0.1.0 staleness caveat is gone |
| `game.native_broadcast` / `native_feedback` | `Messages::broadcast` / `Commands::feedback` | both return `Result`, ignored as a closed connection is not a backup failure |
| `static CORE: Mutex<Option<Core>>` | unchanged | `Hooks: Send + Sync`, so a `Mutex`, never a `RefCell` |
| `log::warn!` (config, retry, save failure) | dropped | the SDK exports no logger and the port adds no crate; config warnings are printed by `/simplebackups status`, the other failures reach `last_error` (also in `status`) and the broadcast |

A hook with **no home**: none. All seven injections and both modules have a typed destination.

### The ladder (measured)

Every compile ran in a `ports/tools/build-slot.sh` lease with `CARGO_INCREMENTAL=0`; the
build/check/fits legs ran locally (`BUILD_LOCAL=1`, trap 5) and the test/clippy/fmt legs on the
remote runner. Evidence in `../_shared/evidence/simplebackups/`.

| Check | Result |
|---|---|
| `cargo test --offline` | **23 passed, 0 failed** (16 `runtime`, 7 `zip`) — `sdk-test.log` |
| `cargo clippy --all-targets -- -D warnings` | exit 0, 0 warnings — `sdk-clippy.log` |
| `cargo fmt --check` | clean, no diff — `sdk-fmt.log` |
| `rm -f Cargo.lock && cargo horizon-mod build` | `Built ./target/horizon-mod/simplebackups.hmod`, exit 0 — `sdk-build.log` |
| second build | **identical bytes**, 29,207 B, sha256 `1d565c1d…4f38` — `sdk-sha256{,-second}.txt` |
| `cargo horizon-mod check simplebackups.hmod` | `simplebackups 0.2.0 (native-mod, format native-1/0b14f8a…/e59da90…, for 26.3)`, 29,207 bytes, exit 0 — `sdk-check.log` |
| `cargo horizon-mod fits simplebackups.hmod --build nightly` | `0b14f8a  made for`, exit 0 — `sdk-fits.log` |
| `unzip -l` | **10 entries** — `horizon-mod.toml`, `Cargo.toml`, `LICENSE`, `LICENSE-NOTICE.md`, `README.md`, `src/{lib,runtime,support,zip}.rs`, `uses.json`; **no `Cargo.lock`** — `sdk-entries.log` |
| packaged `src/runtime.rs` | sha256 `34d0d7…` = the retired file — `sdk-runtime-sha256.txt` |
| `uses.json` | 174 surface entries: 15 `hook:*` refs (the 7 implemented hooks plus their event fields, and `command` — a false positive, trap 8), 22 methods, 14 traits, 36 handles, 84 types, 1 capability (`registries`), 1 macro — `sdk-uses{.json,-summary.log}` |

### Traps this migration hit (new to this port)

1. **The `zip` crate is out.** The bundle compiled inside `horizon-client`, whose lock allowed
   `zip`; an SDK mod may add only `horizon-mod-api` (any other registry crate needs
   `horizon-launcher native-lock`, which is not installed). So `src/zip.rs` writes the archive with
   `std`: stored entries, CRC-32, central directory, and ZIP64 records past 4 GiB or 65535 entries.
   The tests prove it with an independent in-crate reader **and** CPython's `zipfile`
   (`testzip()` checks every CRC); a zip64 case is forced and read back the same way.
2. **No `WorldPhase::WorldOpenReady` at this pin** (only `WorldOpenFailed`, `WorldCloseFinal`,
   `WorldCloseFailed`). The 0.1.0 `world_lifecycle` open branch cannot compile as written: the world
   is taken lazily at the first running `ServerTickEnd` via `host.world_dir()`, and every close
   phase forgets it.
3. **There is no synchronous `save_game()`.** `LevelRef::request_save` asks for a save that runs at
   the *start of the next tick* and reports through `level_save`/`world_saved`. The copy therefore
   starts from `world_saved` (`waiting_save` in the state), and `save_seq` is read *after* the save
   completed — exactly the sequence 0.1.0 read after its synchronous save. A save that never reports
   back is given up after `SAVE_WAIT_TICKS` (200) so the schedule cannot stall; `in_flight` stays
   true while waiting, so a queued manual request still queues.
4. **`host.levels()` borrows the host immutably, `level.request_save(host)` needs it mutably.**
   Collect `list_levels()` into a `Vec<LevelRef>` first, then request.
5. **`cargo horizon-mod` exists here but not on the remote build box** (`error: no such command`).
   The ladder splits: `test`/`clippy`/`fmt` forwarded, `build`/`check`/`fits` with
   `BUILD_LOCAL=1`. Also, the remote runner already starts **inside the port**, so the brief's
   `cd ports/simplebackups` inside the command fails with *No such file or directory* — drop the
   `cd` for forwarded legs.
6. **The remote pull only brings back `*.hmod`, `*.md`, `*.json`, `*.log`** (never `*.rs`, `*.toml`,
   `target/`). Source edits and formatting must happen locally; a build must `cp
   target/horizon-mod/<id>.hmod ./` so the artifact comes back at all.
7. **`cargo horizon-mod check <file> --uses` printed only the header** in this tool build; the
   reliable route is `unzip -p <id>.hmod uses.json`.
8. **`uses.json` is identifier-matched, not a call graph.** `Path::display()` lands as
   `DisplaysServerHost::display`, `event.command` as `hook:command` + `Hooks::command`, and items
   the crate never calls are listed. Read it as "surface the source names", not as calls made.
9. **The packer takes root `README.md` and `LICENSE*`** (and `notes.md`/`VALIDATION.md` are *not*
   packed). The first build shipped the retired template README (8,965 B); it is rewritten for the
   package (3,250 B), and `LICENSE-NOTICE.md` travels with the code.
10. **Clippy on new glue:** a hand-written `Default` matching every field's default is
    `derivable_impls` (derive it), and `len() % 8 == 0` is `manual_is_multiple_of`. Both were fixed
    rather than allowed.
11. **`cargo fmt` runs on the four new/edited files only** (`src/{lib,support,zip}.rs`); never on
    `src/runtime.rs`, which must stay byte-identical, and never `--all` (that reformats the fetched
    SDK).
12. **`.git` at the package root was accepted** by `cargo horizon-mod build` (a hidden directory),
    while `mixins/` and `tools/audit/` were the paths that had to go. The nested repository was
    moved to `evidence/simplebackups/retired-hnmod-0.1.0/port-git` anyway, as every migrated port
    has none, so the root holds package paths and documents only.

### Behaviour that changed with the format (deliberate, and why)

- **Backups are stored, not deflated** (trap 1). They are the size of the world; `compression_level`
  is still parsed and reported by `status`, but has no compressor behind it. A stored ZIP is what
  every reader accepts, and it is the only honest option without a crate.
- **The save is asynchronous** (trap 3): the copy starts one save-cycle later than 0.1.0's
  synchronous `save_game()`. `retry_on_save` still means "a save that lands *inside* the copy makes
  it stale", and the pause-overlap differential is unchanged.
- **No log lines** at all; config warnings appear as `config: …` lines after the four `status` lines
  (which stay byte-identical to 0.1.0, so the existing assertions still match).
- **Non-regular files are skipped** in the walk: a fifo or socket in a world folder would block the
  worker for ever, which is a hang, not a backup.
- **The world folder is `Some` only while a world is open**, so the 0.1.0 warning about a stale
  `native_world_dir` no longer applies.

### Gaps after the migration

1. **No in-game re-run in this environment.** The e2e needs a game the launcher built with this
   package (`horizon-launcher native-build … --trust-native-code`) and `horizon-launcher` is not
   installed here, so **no behaviour claim is made for 0.2.0**. `tools/e2e.sh` is updated for that
   game (`GAME=`, optional `CONTROL_GAME=` for the unmodded control leg, `SELFTEST=1` to prove the
   scheduled-zip assertion is not vacuous) and must be run before the port can claim `ported`.
   The 0.1.0 run quoted above remains the behaviour's evidence for the *rules*, which are
   byte-identical; what changed is the transport.
2. **No deflate**, so a backup cannot be shrunk by `compression_level`; a future SDK that allows a
   pinned compressor crate (or ships a `deflate` helper) closes it.
3. **The 0.1.0 harness logs are gone** (`target/horizon-test.log`, `target/G9-e2e.log` lived in
   `target/`, which the disk-hygiene rule cleans); the transcript quoted in this file is the record.
4. Still deferred: Cherished Worlds/mc2discord and regex ignores (**the disk
   cap is done — *Slice 2 (0.2.2)* —, pause/unpause — *Slice 5 (0.2.5)* —, the persisted tick
   schedule — *Slice 6 (0.2.6)* —, incremental chains/merge — *Slice 7 (0.2.7)* —, the
   empty-server policy — *Slice 8 (0.2.8)* —, `latest.log` — *Slice 9 (0.2.9)* — and the empty-server
   interval — *Slice 12 (0.2.10)*; only the regex half of the old combined gap remains, and R9
   forbids the crate it would need**). Disk reservation and sync-copy are `blocked` (slices 10/11).
5. `/simplebackups status` is the only route for a config warning; there is no log file.
6. `side = "both"`, so the mod is installed on a client too; that is required for single player and
   a LAN host (their integrated server belongs to the client installation).

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices, rust-review

### Verification (SDK 0.2.0, 2026-11-18)

| Check | Command | Result |
|---|---|---|
| Pure rules + archive writer | `cargo test --offline` | **23 passed**, 0 failed (`evidence/simplebackups/sdk-test.log`) |
| Lints | `cargo clippy --offline --all-targets -- -D warnings` | exit 0, 0 warnings (`sdk-clippy.log`) |
| Formatting | `cargo fmt --check` | clean (`sdk-fmt.log`) |
| Package | `rm -f Cargo.lock && cargo horizon-mod build` | `target/horizon-mod/simplebackups.hmod`, exit 0 (`sdk-build.log`) |
| Reproducible | second `build` + `cmp` | identical 29,207 B, sha256 `1d565c1d…4f38` (`sdk-sha256-second.txt`) |
| Registry check | `cargo horizon-mod check simplebackups.hmod` | exit 0, format `native-1/0b14f8a/e59da90`, for 26.3 (`sdk-check.log`) |
| Fits the pinned build | `cargo horizon-mod fits simplebackups.hmod --build nightly` | `0b14f8a  made for`, exit 0 (`sdk-fits.log`) |
| Entries | `unzip -l simplebackups.hmod` | 10 entries, no `Cargo.lock` (`sdk-entries.log`) |
| `runtime.rs` untouched | `unzip -p … src/runtime.rs \| sha256sum` | `34d0d7…` = the retired file (`sdk-runtime-sha256.txt`) |
| Surface | `unzip -p … uses.json` | 174 entries, 7 hooks + `registries` (`sdk-uses-summary.log`) |
| Skills line | `python3 ports/tools/check-skills.py --notes ports/simplebackups --strict` | passes |

### Changelog (0.2.0)

> **0.2.0 — first publication, as an SDK `.hmod`.** Horizon's mod-API merge (launcher ≥ 0.2.70)
> retired `.hnmod`, so Simple Backups is now one flat `simplebackups.hmod` (29,207 B): `horizon-mod.toml`
> with `api = "0.2"`, `src/lib.rs` as the one `impl mod_api::Hooks`, and `src/runtime.rs`
> byte-identical to 0.1.0, so the schedule, the UTC names, the retention and the
> save-overlap retry behave exactly as before. New for `api = "0.2"`: the config is read through the
> host's own `config/simplebackups.toml` service (warnings now reach `/simplebackups status`), the
> world folder comes from `host.world_dir()`, the save is a `LevelRef::request_save` whose copy
> starts when the save cycle completes, the commands are typed (`Command::op` +
> `run_mod_command` with a `boolean` argument), and the archive is written by the port itself with
> `std` — the `zip` crate a mod cannot depend on is gone, so backups are stored rather than
> deflated. The `.hnmod`, the mixin plan, the two modules and the seven per-hook helpers are gone.
> Built against SDK `0b14f8a` / mixins `e59da90` for 26.3; same feature set and the same gaps, plus
> no compression. Not yet re-run in game (no launcher here).

### Changelog (0.2.2)

> **0.2.2 — the disk cap.** `config/simplebackups.toml` gains `max_disk_size` (upstream's
> `maxDiskSize`, default `"25 GB"`, binary units). After a good backup the port removes this world's
> oldest `<world>_*.zip` archives while the backup folder is over that cap, always keeping the newest
> one, and `/simplebackups status` reports `disk=<size> cap=<size> over=<bool>`; when one archive is
> left and still too big the completion message says so. Foreign files in the folder count toward the
> total but are never deleted. Repinned to Horizon `0a875da` (mixins unchanged), 36 tests, and the
> new scene is proven in game — a 1 B cap leaves exactly the newer archive, a 1 TB counterfactual
> leaves both. Package 38,169 B, sha256 `2edec944…`.

## Slice 9 (0.2.9, 2026-11-24) — capture `latest.log` as `backup.log`

**Gap line.** The coverage table above: *"`latest.log` → `backup.log` | not ported | gap (needs a
path outside the world folder)"*, and *Gaps after the migration* item 4. Upstream's
`captureLatestLog` (`CommonConfig.java:65-66`, default true) makes `BackupThread.java:469-498`
reopen the finished archive and append the game log as a flat `backup.log`. Class **server-only**,
tier **logic** (ledger authority): one config key, one pure entry-name rule, one guarded glue read
after the world walk — no new hook, crate (R9), block entity, menu or carried content.

**First measurement — the log path (done before any code).** A headless launch in a fresh
`E2E_TMP` scratch home with the exact `launcher-game.sh env simplebackups` environment printed:

```
SCRATCH=/home/adlf/.cache/horizon-e2e/scratch/simplebackups-logprobe.046XZl
$SCRATCH/config/simplebackups.toml
$SCRATCH/game/logs/latest.log          (1,240 B)
$SCRATCH/game/logs/.lock
$SCRATCH/saves/LogProbe/level.dat
$SCRATCH/options.txt
$SCRATCH/game/: logs/                   # no saves here
```

`HORIZON_HOME=$scratch/game`; the working/instance directory (`host.instance_dir()`, where
`options.txt` and `saves/` live) is `$scratch`. The production launcher sets
`cmd.current_dir(&instance.home).env("HORIZON_HOME", &instance.home)`
(`launcher/src/instances.rs:661-663`), so there `instance_dir()/logs/latest.log` **is** the log; the
harness sets `HORIZON_HOME` beside the instance, so the launcher's own variable is a second measured
root. `ConfigFiles` itself is bound to the mod's `config/` and refuses `..` and anything it does
not own (`content/config_files.rs:18-37,114-121`), so it cannot name the log; the path-bearing
method in the same `ConfigHost` area is `instance_dir()`. No absolute path is hardcoded: the glue
tries `instance_dir()/logs/latest.log` and `$HORIZON_HOME/logs/latest.log`, in that order.

**Rules (pure, `src/runtime.rs`).** `BACKUP_LOG_ENTRY = "backup.log"`,
`GAME_LOG_RELATIVE = "logs/latest.log"`, `backup_log_entry(name)` accepts only that exact flat name
(a nested or `..` name is refused), and `game_log_path(home, relative)` accepts only the exact
relative name through the existing `normalise` (absolute paths and parent components refused).
`Settings` gains `capture_latest_log` (default true, upstream's spelling) with parse, warning and
round-trip. Glue (`src/support.rs`): `game_log_homes` derives the two roots on the host thread;
`read_one_game_log` refuses a symlink at the file, a symlinked parent that resolves out of the
home, and any path not `file_type().is_file()`; the worker appends `backup.log` after the world
walk when the key is on. A missing log is not a backup failure — it just means no entry.

**Red → green.** Four failing tests first (`sdk-test-red-latestlog.log`, verbatim):

```
---- runtime::tests::a_game_log_path_may_not_leave_the_game_folder stdout ----
thread 'runtime::tests::a_game_log_path_may_not_leave_the_game_folder' (663270) panicked at src/runtime.rs:2019:9:
assertion `left == right` failed: parent traversal must be refused
  left: Some("/game/logs/latest.log")
 right: None

---- runtime::tests::the_game_log_is_stored_under_one_flat_entry_name stdout ----
thread 'runtime::tests::the_game_log_is_stored_under_one_flat_entry_name' (663312) panicked at src/runtime.rs:1978:9:
assertion `left == right` failed: the log is a flat entry beside the world tree
  left: Some("backup.log")
 right: None

---- runtime::tests::the_log_capture_key_defaults_on_and_round_trips stdout ----
thread 'runtime::tests::the_log_capture_key_defaults_on_and_round_trips' (663313) panicked at src/runtime.rs:1992:9:
upstream captureLatestLog defaults to true

---- support::tests::the_game_log_is_read_from_a_safe_path_only stdout ----
thread 'support::tests::the_game_log_is_read_from_a_safe_path_only' (663328) panicked at src/support.rs:2140:9:
assertion `left == right` failed: the exact bytes of latest.log are captured
  left: None
 right: Some([91, 49, 48, 58, 52, 54, 58, 51, 52, 93, ...])

test result: FAILED. 73 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
```

Green: **77 passed, 0 failed** (`sdk-test.log`), clippy `-D warnings` 0 warnings (`sdk-clippy.log`),
`fmt --check` clean (`sdk-fmt.log`). Clippy caught two real nits (a collapsible `if` and three
`[home.clone()]` slices in tests); both were fixed, not allowed.

### Package 0.2.9

| | |
|---|---|
| version | **0.2.9** (`horizon-mod.toml` + `Cargo.toml`) |
| bytes | **65,996 B** |
| sha256 | `53a767bb1c2a55915391ea69630fd4f36bad5a30ec8727bac820e2ad8bb31606` |
| entries | **10** — `horizon-mod.toml`, `Cargo.toml`, `LICENSE`, `LICENSE-NOTICE.md`, `README.md`, `src/{lib,runtime,support,zip}.rs`, `uses.json`; **no `Cargo.lock`** |
| reproducible | two builds, each after `rm -f Cargo.lock`: `cmp` identical, same sha (`sdk-cmp.log`) |
| `check` | `simplebackups 0.2.9 (native-mod, format native-1/0a875da…/e59da90…, for 26.3)`, 65,996 bytes, exit 0 |
| `fits --build 0a875da91dbb0fdcb7ba8c42c11341aa229af51a` | `0a875da  made for` |
| `fits --build 0b14f8acfd97fa2c750e42795249dcc981178ca4` | `0b14f8a  fits` — the real gate |
| `verify-sdk-package.py simplebackups` | `1/1`, `package matches the tree`, R9 clean |

**In game.** Launcher build `native-0a875da-d81f6f42e8b4`, `Compiling simplebackups (1 of 1)`,
every packed source byte-identical to this tree (`../_shared/evidence/simplebackups-e2e/build-record.txt`).
Scenario M's transcript (`e2e-launcher.log`):

```
game log (/home/adlf/.cache/horizon-e2e/scratch/simplebackups-gamelog.OCsB83/game/logs/latest.log) line: [11:12:39] [Server thread/INFO]: Starting integrated minecraft server version 26.3
zip entries: 14
backup.log holds the exact line chosen from the scratch latest.log
== scenario M counterfactual: capture_latest_log=false packs no backup.log ==
game log (.../simplebackups-gamelog.fsp95s/game/logs/latest.log) line: [11:12:49] [Server thread/INFO]: Starting integrated minecraft server version 26.3
counterfactual: capture_latest_log=false packed no backup.log
PASS: … a concrete line from the scratch latest.log is captured as a flat backup.log entry and capture_latest_log=false packs no such entry (scenario M) …
EXIT:0
```

`SELFTEST=latestlog` (`e2e-launcher-gamelog-selftest.log`) inverts the entry assertion and ends
`FAIL: GAMELOG SELFTEST: backup.log held the chosen line — the assertion is alive` + `EXIT:1`.
All twelve earlier families (A–L/L2) stayed green on the 0.2.9 game; the control game's leg fails
at scenario A (`EXIT:1`), `SELFTEST=1`, `SELFTEST=pause`, `SELFTEST=ticks`, `SELFTEST=chain`,
`SELFTEST=noplayer`, `STALE_SELFTEST=1`, `ADOPT_NEGATIVE=1`, `SAME_SECOND_NEGATIVE=1` all `EXIT:1`.
`check-e2e-logs.py simplebackups-e2e --strict` → `logs=14 ok=14 missing-exit=0 bad-exit=0`.

**Remote build path.** Every cargo ladder step forwarded through
`ports/tools/build-slot.sh run --job W1-simplebackups --port ports/simplebackups --step …` to host
**AdLf-PC** (`build-slot: forwarding … to the remote runner`); `BUILD_LOCAL=1` was never set. The
launcher native build and every game leg stayed local (`FORWARDED=1` on the launcher build), because
the second box has no launcher/Nightly cache. All e2e scratch dirs came from `E2E_TMP` under
`~/.cache/horizon-e2e/scratch` — never `/tmp`.

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices, rust-review

### Changelog (0.2.9)

> **0.2.9 — the game log goes into the backup.** `capture_latest_log` (upstream's
> `captureLatestLog`, default true) appends the game's `logs/latest.log` to every archive as one flat
> `backup.log` entry, after the world walk. The path is resolved from the game's own directory —
> `host.instance_dir()` (the launcher's production layout) and the launcher's `HORIZON_HOME` (the
> harness layout) — never from a hardcoded absolute path; a symlink, an absolute path, `..` and any
> path that canonicalises outside that root are refused. The config key, a flat entry-name rule and
> the guarded read are all that changed; no new hook, crate, block entity or menu. 77 tests;
> scenario M proves an exact line from the scratch log is in the archive, the key-off scene packs no
> entry, and `SELFTEST=latestlog` proves the assertion can fail. 65,996 B, sha256 `53a767bb…`.

## Slice 10 (2026-11-24) — disk reservation before a copy: `blocked(no-disk-space-api)`

**Gap line.** Coverage table: *"disk reservation before a copy | not ported | gap (upstream's
`NotEnoughDiskSpaceException`)"*. Class **server-only**, tier **logic**: one reserve key, one pure
required-bytes rule and one guard before spawning the worker; no new hook, crate, block entity or
menu. The shape fits the tier; the **pin does not expose the measurement the guard needs**, so the
slice stops at the first measurement gate rather than guessing a size or faking a guard.

**Upstream rule, read first from the pack's source** (branch `1.21.1`, head
`bf04de3e7b3e05504ee5cc974cf45833b0d4b9a4`, re-cloned read-only to
`ports/_shared/upstream/simplebackups-src` and deleted after these findings):

- `BackupThread.java:56` — `public static final long BACKUP_BUFFER_SIZE = 128L * 1024 * 1024;
  // 128 MB`. This is the whole reserve: a fixed 128 MiB, **not** a config key upstream exposes.
- `BackupThread.java:374` — `FileStore fileStore = Files.getFileStore(outputFile);` — the usable
  space is queried from the **filesystem of the output file**, through Java NIO's `FileStore`.
- `BackupThread.java:395-398` — for each file about to be zipped:
  `fileStore.getUsableSpace() - attrs.size() - BACKUP_BUFFER_SIZE < 0L` →
  `throw new NotEnoughDiskSpaceException("Not enough space on disk to create backup")`. The guard is
  **per file**, and the required bytes are that file's own size plus the fixed 128 MiB buffer.
- `BackupThread.java:225-227` — the catch broadcasts `simplebackups.not_enough_space` and deletes
  the partial `backupFilePath`, leaving earlier archives intact.
- `exception/NotEnoughDiskSpaceException.java` is an `IOException` subtype; there is no retry or
  fallback path.

So the faithful rule is: `usable_space(output volume) >= next_file_size + 128 MiB`, checked before
each file is copied; a failure publishes no archive and reports the reservation error.

**First measurement gate — FAILED (no free-space source reachable under R9).** Evidence logged in
`../_shared/evidence/simplebackups/reserve-gate-0a875da.log`:

1. The port's only dependency is `horizon-mod-api` (`Cargo.toml`), so R9 permits no `libc`/`nix`/
   `statvfs` crate.
2. A case-insensitive search of the whole pinned SDK (`ports/_shared/sdk/0a875da/crates/`, 1,636
   files, and `surface.json`, 6.8 MB) for `statvfs|statfs|f_bavail|f_bfree|get_usable_space|
   usable_space|free_disk|disk_free|free_space|available_space|disk_capacity` returned only two
   `available_space` hits, both the unrelated GUI field `ScreenRenderInventoryMobEffectsEvent.
   available_space: i32` (`surface.json:45375`, `hooks/screen_preedit_effects.rs:32`). No disk or
   filesystem-capacity method exists in the host surface.
3. The only `space_for` in the SDK is `ItemStorage::space_for(host, &ItemResource) ->
   Result<u64, StorageError>` (`wire_guest_generated.rs:1883`, `horizon-mod-vocab/src/item/storage.rs:144`)
   — free **item** slots in an inventory, not bytes on a volume.
4. The only space/path-bearing host methods found are `native_config_dir`, `native_data_dir`,
   `native_data_pack_files`, `native_world_dir`; none reports capacity.
5. Rust `std` (rustc 1.97.0) has no free-space query: `std::fs::free_space(".")` fails to compile
   (`E0425, cannot find function free_space in module std::fs`), and no `statvfs` binding exists in
   the shipped std source. `MetadataExt::blocks()/blksize()` compile but describe a single file's
   allocation, never free filesystem bytes.

**Decision.** `blocked(no-disk-space-api)`. There is no honest value to plug into
`usable >= size + 128 MiB` at this pin: a configured "reserve" without a measured free-space value
cannot guard anything, and a write-to-failure probe would mutate the volume and is not a measurement.
Per the brief, the slice stops here instead of guessing a size or dressing a chat command up as a
reservation test. **No code, config, version, manifest or package changed** — the delivered artifact
is still 0.2.9 (65,996 B, sha256 `53a767bb1c2a55915391ea69630fd4f36bad5a30ec8727bac820e2ad8bb31606`;
`fits --build 0a875da91dbb0fdcb7ba8c42c11341aa229af51a` → `made for`,
`fits --build 0b14f8acfd97fa2c750e42795249dcc981178ca4` → `fits`). The 0.2.4–0.2.9 rules are
untouched, so none can regress. The in-game assertion (a reserve larger than scratch free space
publishes no `.zip` and the newest `status` line reports the error; zero reserve publishes one) is
**not runnable** here: without a free-space query the guard cannot exist, and a synthetic
`SELFTEST=reserve` would be a fake.

**Unblock condition.** A pinned `horizon-mod-api` method that returns the usable bytes of a volume
(or an equivalent `statvfs`-style helper the launcher accepts) would let the slice proceed: add
`reserve_bytes = "128 MB"` (upstream's fixed buffer, made configurable here), the pure
`required_bytes(file_size, reserve)` rule red-first, the per-file guard before the worker copies,
and the fail-capable two-direction `ctl chat` scene. Re-check this at every new pin.

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices

## Slice 11 (2026-11-24) — sync-copy mode (`copy_on_server_thread`): `blocked(no-sync-observable)`

**Gap line.** *Deferred (notes.md, upstream source)*: "Cherished Worlds, mc2discord, regex
ignores, sync-copy mode, and upstream's separate `noPlayerBackupTimer`". Class **server-only**,
tier **logic** (ledger authority): one config key plus a branch choosing whether the copy runs
inline at the save-complete moment or on the current worker thread — no new hook, block entity,
menu, crate or carried content. The shape fits the tier. **The first gate fails**: the harness
cannot distinguish the two thread placements within one `ctl` round trip, so the slice stops
before adding a config key whose only effect would be unobservable.

**The branch as read, upstream first.** `BackupThread.java:356-363` (`1.21.1` head
`bf04de3e7b3e05504ee5cc974cf45833b0d4b9a4`, recorded in `ports/_shared/upstream/simplebackups/
FINDINGS.md` §1.4/§1.7) runs the save with `server.executeBlocking(() ->
server.saveEverything(true, false, true))` — the save **blocks the server thread**, then the zip,
log capture, retention and disk accounting run on the `Thread` named `SimpleBackups`. Upstream
has **no** `copy_on_server_thread` key (the config inventory in §1.1 has none); the mode is a
campaign idea from §3/Q3 of the same findings — "copy synchronously on the server thread … at the
cost of a tick stall" — which the brief makes the next implementable gap. In this port the same
shape is: `perform_command` queues `pending` (`support::run_command`); `ServerTickEnd` takes it
and `start_backup` asks for a save (`request_save`); `world_saved` calls `begin_copy`, which
spawns the worker (`support.rs:683`, `thread::spawn`); the worker writes `.zip.part` and the next
`ServerTickEnd` publishes it. The flag would move the `run_backup` call from that worker into
`on_world_saved` (the save-complete moment) while leaving the *moment* unchanged.

**First measurement gate — FAILED (no in-one-round-trip observable).** Evidence logged in
`../_shared/evidence/simplebackups/sync-copy-gate-0a875da.log` (probe
`tools/probe-synccopy.sh`, 0.2.9 game, 128 MiB `payload.bin`, `save_before_backup=true`,
`compression_level=1`, schedule disabled, scratch under `E2E_TMP`):

```
payload-bytes: 134217728
before-status: saves=0 retries=0 stale=false paused=false ticks=42 chain=0 players=1 empty_backups=0
backup-chat-reply: {"ok":true}
backup-chat-seconds: 0.044
immediate-list-seconds: 0.379
immediate-list-replies: Backup queued. disk=0 B ... saves=0 ... in_flight=false last=- error=- ...
archives-at-immediate-list: 0
archive-appears-after-backup-reply-seconds: 10.634
archive-count: 1
after-status: ... ticks=216 chain=1 ...
```

Three independent facts make the proposed assertion impossible to satisfy honestly:

1. **`ctl chat "/simplebackups backup"` returns in 44 ms** with `{"ok":true}` — the reply is the
   command dispatcher's acknowledgment, not the backup's. `perform_command` only queues a
   request; the copy is never executed in that hook (the port's own design note forbids saving
   inside a player-state-swapped command hook). No placement of the copy — inline or worker —
   can make this call wait for the archive, so "must not return until the archive exists" cannot
   be true of sync mode either.
2. **The copy starts after the reply, at save-complete.** The immediate `list` (379 ms) still
   reads `ticks=42`, `in_flight=false` and names **0** archives — the `ServerTickEnd` that starts
   the request and the save that completes it have not happened yet in that window. An inline
   copy at `world_saved` has not begun, so the immediate `list` cannot see it even with a huge
   payload.
3. **The payload is large enough; the gap is not size.** At 128 MiB the archive appeared 10.634 s
   after the chat reply — ~28× the whole immediate-list window and ~240× the chat round trip.
   A `wait`-timing difference would only be visible if a `ctl` command were processed *during* an
   inline copy, which the queued command flow does not arrange; dressing that latency up as
   "the command waited for the archive" would be false evidence.

**Decision.** `blocked(no-sync-observable)`. Per the brief, the gate stops the slice here rather
than inventing an observable. **No code, config, version, manifest or package changed** — the
delivered artifact is still 0.2.9 (65,996 B, sha256
`53a767bb1c2a55915391ea69630fd4f36bad5a30ec8727bac820e2ad8bb31606`;
`fits --build 0a875da91dbb0fdcb7ba8c42c11341aa229af51a` → `made for`,
`fits --build 0b14f8acfd97fa2c750e42795249dcc981178ca4` → `fits`), `verify-sdk-package.py` → `1/1`
(`package matches the tree`), and `check-e2e-logs.py simplebackups-e2e --strict` → `logs=14 ok=14`
so the 0.2.4–0.2.9 rules cannot have regressed. The pure branch would be trivial and therefore
uninformative: its only effect is a thread switch, whose observable difference is server-thread
stall time, not a game state an assertion could read.

**Unblock condition.** A harness route that drives the backup **synchronously through the command
reply** (so the copy really begins before the `ctl chat` returns), or a game-visible marker for
"this copy ran on the server thread" (a status field set at the save-complete moment that a
`ctl` round trip can read after the copy), would make the two placements distinguishable. With
either, add `copy_on_server_thread` (default `false`, today's behaviour), the red-first pure rule,
the inline branch, and a `SELFTEST=synccopy` inversion. Re-check this gate at every new pin.

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices

## Slice 12 (0.2.10, 2026-11-24) — the empty-server interval (`noPlayerBackupTimer`)

**Gap line.** The *Deferred* bullet (and *Gaps after the migration* item 4): *"Cherished Worlds,
mc2discord, regex ignores, sync-copy mode, and upstream's separate `noPlayerBackupTimer`"* — this is
the last implementable family there. Class **server-only**, tier **logic** (ledger authority):
one `Settings` key, one pure `empty_period_interval` rule, one branch in the existing wall-clock
due gate, and one field on the existing `saves=` status line. No new hook, crate, block entity,
menu, carried content or registry dependency. Disk reservation stays `blocked(no-disk-space-api)`
(slice 10) and sync-copy `blocked(no-sync-observable)` (slice 11) — both measured, not retried;
regex ignores stay `not ported` (R9 forbids the crate).

**Artefact rule, read first** from `CommonConfig.java` at the pack's `1.21.1` head
`bf04de3e7b3e05504ee5cc974cf45833b0d4b9a4` (fetched read-only from the public source; the
earlier clone was deleted after analysis):

- `:90-91` — `noPlayerBackupTimer = builder.comment("The time between two backups when no player is
  online. 0 means the same as the \"timer\" setting.").defineInRange("noPlayerBackupTimer", 0, 0,
  Integer.MAX_VALUE)` — default **0**, in **minutes**, 0 meaning "use `timer`".
- `:151-157` — `getTimer(boolean arePlayersOnline)`: `int i = noPlayerBackupTimer.get(); if (i == 0)
  { i = timer.get(); } return (long)(arePlayersOnline ? timer.get() : i) * 60 * 1000;`

So the effective minutes are `online ? timer : (empty == 0 ? timer : empty)`, and the millisecond
interval is minutes × **60 × 1000**. The port's existing `due_by_millis` already multiplies by
60_000, so no new conversion constant was introduced; `interval_ticks > 0` stays the test override
and is untouched. Upstream's tick-counter mode derives from `timerMinutes * 20 * 60`
(`BackupThread.java:115-121`), which is the same minutes value fed to the same gate.

**Port decisions.** `Settings` gains `no_player_backup_timer_minutes: u64` (default 0); `parse`
accepts the key as an unsigned integer (a bad value keeps the default and warns) and `to_toml`
writes it back. `runtime::empty_period_interval(players_online, main_minutes, empty_minutes)`
encodes upstream's selection. The wall-clock branch of the due gate in `support::on_tick` now uses
`empty_period_interval(players > 0, interval_minutes, no_player_backup_timer_minutes)` instead of the
raw `interval_minutes`, so an explicit empty timer replaces the interval only while nobody is online
and 0 reuses the main one. `/simplebackups status` appends `timers=<configured>` to the existing
`saves=` line; every earlier substring assertion stays valid. The 0.2.2–0.2.9 behaviour is otherwise
unchanged.

**Red → green.** Three failing tests first, against a deliberately wrong stub (it ignored an
explicit empty timer and let 0 fall through to 0), verbatim from `sdk-test-red-noplayertimer.log`:

```
---- runtime::tests::a_zero_empty_timer_falls_back_to_the_main_interval stdout ----
thread 'runtime::tests::a_zero_empty_timer_falls_back_to_the_main_interval' (828647) panicked at src/runtime.rs:2076:9:
assertion `left == right` failed
  left: 0
 right: 120

---- runtime::tests::an_explicit_empty_timer_replaces_the_main_interval stdout ----
thread 'runtime::tests::an_explicit_empty_timer_replaces_the_main_interval' (828649) panicked at src/runtime.rs:2070:9:
assertion `left == right` failed
  left: 120
 right: 7

---- runtime::tests::the_no_player_timer_key_defaults_to_zero_and_round_trips stdout ----
thread 'runtime::tests::the_no_player_timer_key_defaults_to_zero_and_round_trips' (828688) panicked at src/runtime.rs:2090:9:
the key must be known: ["unknown key: no_player_backup_timer"]

test result: FAILED. 77 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

Both `empty_period_interval` branches are proved red-first there: explicit timer (`false,120,7 → 7`)
and 0 fallback (`false,120,0 → 120`, plus the online wins case). Green: **80 passed, 0 failed**
(`sdk-test.log`), clippy `-D warnings` 0 warnings (`sdk-clippy.log`), `fmt --check` clean
(`sdk-fmt.log`). `cargo fmt` first found two real diffs (the new function signature and the import
list); both were formatted, not allowed.

**In game.** Launcher build `native-0a875da-b8e99fd6e39a`, `Compiling simplebackups (1 of 1)`,
every packed source byte-identical to this tree (`build-record.txt`). Scenario N's transcript
(`e2e-launcher.log`):

```
== scenario N: the parsed newest status line reports the configured no_player_backup_timer ==
no-player-timer status (newest): saves=0 retries=0 stale=false paused=false ticks=24 chain=0 players=1 empty_backups=0 timers=7
parsed newest status line: timers=7 (configured no_player_backup_timer=7)
harness: players=1; a genuinely playerless interval is not producible by ctl — both empty_period_interval branches are proved red-first by unit tests, not dressed up as a chat command
PASS: … the parsed newest status line reports timers=<configured no_player_backup_timer> (scenario N) …
EXIT:0
```

`SELFTEST=noplayertimer` (`e2e-launcher-noplayertimer-selftest.log`) inverts the one parsed value and
ends `FAIL: NOPLAYERTIMER SELFTEST: the newest status line reported timers=7 — the assertion is alive`
+ `EXIT:1`. **Harness limit (measured, as in slice 8):** `ctl launch` opens a single-player
integrated world whose owner is placed (`players=1` in every N/L/L2 run), so a genuinely playerless
interval cannot be produced; no chat command is presented as one. All fourteen previous scenario
families stayed green on the 0.2.10 game; control `EXIT:1`, `SELFTEST=1`, `SELFTEST=pause`,
`SELFTEST=ticks`, `SELFTEST=chain`, `SELFTEST=noplayer`, `SELFTEST=latestlog`,
`SELFTEST=noplayertimer`, `STALE_SELFTEST=1`, `ADOPT_NEGATIVE=1`, `SAME_SECOND_NEGATIVE=1` all
`EXIT:1`. `check-e2e-logs.py simplebackups-e2e --strict` → `logs=15 ok=15 missing-exit=0 bad-exit=0`.

**Remote build path.** Every cargo ladder step forwarded through
`ports/tools/build-slot.sh run --job W1-simplebackups --port ports/simplebackups --step …` to host
**AdLf-PC** (`build-slot: forwarding … to the remote runner`); `BUILD_LOCAL=1` was never set. The
launcher native build and every game leg stayed local (`FORWARDED=1` keep-local marker on the
launcher build), because the second box has no launcher/Nightly cache. All e2e scratch dirs came
from `E2E_TMP` under `~/.cache/horizon-e2e/scratch` — never `/tmp`.

### Package 0.2.10

| | |
|---|---|
| version | **0.2.10** (`horizon-mod.toml` + `Cargo.toml`) |
| bytes | **66,949 B** |
| sha256 | `ab6e2312d30142d2ae45476c735abd8505ec790668a1d2c01d86df62f608b737` |
| entries | **10** — `horizon-mod.toml`, `Cargo.toml`, `LICENSE`, `LICENSE-NOTICE.md`, `README.md`, `src/{lib,runtime,support,zip}.rs`, `uses.json`; **no `Cargo.lock`** |
| reproducible | two builds, each after `rm -f Cargo.lock`: `cmp` identical, same sha |
| `check` | `simplebackups 0.2.10 (native-mod, format native-1/0a875da…/e59da90…, for 26.3)`, 66,949 B, exit 0 |
| `fits --build 0a875da91dbb0fdcb7ba8c42c11341aa229af51a` | `0a875da  made for` |
| `fits --build 0b14f8acfd97fa2c750e42795249dcc981178ca4` | `0b14f8a  fits` — the real gate |
| tests | **80 passed, 0 failed** (46 `runtime`, 14 `support`, 12 `zip`) |
| `verify-sdk-package.py simplebackups` | `1/1`, `package matches the tree`, R9 clean |

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices

### Changelog (0.2.10)

> **0.2.10 — the empty-server interval.** `no_player_backup_timer` (upstream's
> `noPlayerBackupTimer`, minutes, default 0) gives an empty server its own interval: a nonzero value
> replaces `interval_minutes` while the player list is empty, 0 reuses the main interval, and a
> player online always uses the main interval — upstream's `getTimer(arePlayersOnline)` selection.
> The pure `empty_period_interval` rule and the wall-clock due gate are the only behaviour changes;
> `/simplebackups status` now reports `timers=<configured>` beside the other counters. 80 tests;
> scenario N parses the newest status line for `timers=7`, `SELFTEST=noplayertimer` proves that
> assertion can fail, and both interval branches are proved red-first by unit tests because a `ctl`
> launch always has its owner online. 66,949 B, sha256 `ab6e2312…b737`.

## Slice 13 (0.2.11, 2026-11-24) — bind an adoption to the archive digest

**Source — the human reviewer's exact follow-up on PR #1578 (2026-11-24):** *"The later finding is
about a replacement for an already adopted archive. At `bcb8c188dc7a`, the listing still includes
the same 0.2.4 and 0.2.5 digests. Both match old adoptions by filename and size, which cannot tell
a replacement archive from the one the operator adopted. Please omit those unpublished artifacts
and bind adoption to the archive digest, rechecking it before deletion in their replacement."*
The listing half is the orchestrator's; this slice is the product half. Class **server-only**, tier
**logic** (ledger authority): one pure rule, one extra journal column, two guarded deletion sites
and one extra in-game assertion — no new hook, block entity, menu, renderer, carried content or
registry dependency (R9 still forbids a hash crate, so SHA-256 is implemented with `std` here).

**Gap line.** *Gaps / deviations*: "retention prunes … a marker-less 0.2.0–0.2.2 archive only after
an operator's `/simplebackups adopt <file>`" — an adoption identified by name and size alone cannot
distinguish the file the operator approved from a replacement of the same length. This is a review
fix of the same class as slices 3 and 4, not a new family.

**Code paths as read (before any edit).**
- `src/runtime.rs`: `Adoption { name, bytes }`, `parse_adoptions` (`<bytes>\t<name>`),
  `format_adoptions`, `is_adopted(adoptions, name, bytes)` — name and size only.
- `src/support.rs`: `adopt_archive` records `<bytes>\t<name>` in `simplebackups-adopted.txt`;
  `retainable` = "our marker, or an adopted name+size and a world-shaped ZIP"; `owned_archives`
  plans candidates from that; `publish` and `enforce_cap` delete them with an unguarded
  `fs::remove_file`.

**Rules (pure, `src/runtime.rs`).** `Adoption` gains `sha256: String`; the journal is now
`<bytes>\t<sha256>\t<name>`, and `is_adopted(adoptions, name, bytes, sha256)` requires all three to
match. `sha256_hex` is a std-only SHA-256 (verified against the `""` and `"abc"` standard vectors).
A two-column pre-digest journal line is ignored — an old adoption cannot be verified, so it adopts
nothing until the operator re-adopts. The digest is case-insensitive on read and normalised to
lowercase hex.

**Glue (`src/support.rs`).** `adopt_archive` digests the exact file it adopts and records that
digest (the reply names it). `retainable` recomputes the current file's digest before calling
`is_adopted`, so a replacement of the same name and size is no longer a retention candidate.
`adoption_still_matches` re-checks the digest **immediately before each `remove_file`** in both
`publish`'s `keep` pass and `enforce_cap`'s cap pass — a replacement landing between planning and
deletion is also refused. Marked archives keep their own identity and are unaffected. The status
line's `adopted=<n>` counts journal entries as before.

**Red → green.** Three pure tests were written before the glue, against stubs that dropped the
digest from the journal and ignored it in `is_adopted`; verbatim (`sdk-test-red-adopt-digest.log`):

```
---- runtime::tests::an_adoption_whose_digest_no_longer_matches_refuses_deletion stdout ----

thread 'runtime::tests::an_adoption_whose_digest_no_longer_matches_refuses_deletion' (1446388) panicked at src/runtime.rs:1854:9:
a file replaced under the same name and size must fall out of retention
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- runtime::tests::the_adoption_journal_round_trips_the_digest stdout ----

thread 'runtime::tests::the_adoption_journal_round_trips_the_digest' (1446415) panicked at src/runtime.rs:1838:9:
assertion `left == right` failed: the digest must round-trip: 1234	aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa	W_2026-01-01_00-00-00.zip
42	bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb	W_2026-01-02_00-00-00.zip

  left: [Adoption { name: "W_2026-01-01_00-00-00.zip", bytes: 1234, sha256: "" }, Adoption { name: "W_2026-01-02_00-00-00.zip", bytes: 42, sha256: "" }]
 right: [Adoption { name: "W_2026-01-01_00-00-00.zip", bytes: 1234, sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" }, Adoption { name: "W_2026-01-02_00-00-00.zip", bytes: 42, sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" }]


failures:
    runtime::tests::an_adoption_whose_digest_no_longer_matches_refuses_deletion
    runtime::tests::the_adoption_journal_round_trips_the_digest

test result: FAILED. 81 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
```

The third test, `a_matching_digest_still_permits_deletion`, is the paired guard: it passes under
the stub (a correct digest already matched then) and must stay green — the two failures above are
the ones the fix closes. The glue has its own red-green too: `support::tests::a_replaced_adopted_archive_survives_until_it_is_re_adopted`
exercises adopt → replace in place with same size/different bytes → survive → re-adopt → prune.

Green: **84 passed, 0 failed** (45 `runtime`, 14 `support`, 12 `zip` + the SHA-256 vectors) —
`sdk-test.log`; clippy `--all-targets -D warnings` 0 warnings (`sdk-clippy.log`, after fixing one
`needless_borrow` the new `owned_archives` argument introduced); `fmt --check` clean (`sdk-fmt.log`).

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development,
systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices, rust-review

**In game (launcher build `native-0a875da-4dc9d8895670`, `Compiling simplebackups (1 of 1)`;
every packed source byte-identical to this tree — `../_shared/evidence/simplebackups-e2e/build-record.txt`).**
Scenario G's new leg, from `e2e-launcher.log` (the digests are the actual ones the mod recorded):

```
manual export survived both passes: …/SimpleBackupsAdopt_1999-01-01_00-00-00.zip
adopt reply: adopted SimpleBackupsAdopt_1999-01-01_00-00-00.zip (456 B), digest c2742d745b31cce085fed5b2ee70b2a4c9dc11728bc02b840e03407ef0862e92; retention may now delete it
adopted export pruned by the next publish (the differential)
replaced export under the same name and size (456 B): …/SimpleBackupsAdopt_1999-01-01_00-00-00.zip
replaced export survived the publish (digest no longer matches the adoption)
re-adopt reply: adopted SimpleBackupsAdopt_1999-01-01_00-00-00.zip (456 B), digest 21acf078e852f824d2497e281ca8cd8cc8aa038a0aa2d6210fc4f3dc25449beb; retention may now delete it
re-adopted replacement pruned by the next publish (the differential)
```

The survival assertion is fail-capable: `SELFTEST=adopt-digest` (`e2e-launcher-adopt-digest-selftest.log`)
inverts it and ends `FAIL: ADOPT-DIGEST SELFTEST: the replaced export survived the publish — the
digest assertion is alive` + `EXIT:1`. All 15 earlier legs stayed green on the 0.2.11 game:
`PASS` + `EXIT:0` (A–N incl. M-off, L/L2), control `EXIT:1`, `SELFTEST=1`, `SELFTEST=pause`,
`SELFTEST=ticks`, `SELFTEST=chain`, `SELFTEST=noplayer`, `SELFTEST=latestlog`,
`SELFTEST=noplayertimer`, `SELFTEST=adopt-digest`, `STALE_SELFTEST=1`, `ADOPT_NEGATIVE=1`,
`SAME_SECOND_NEGATIVE=1` all `EXIT:1`. `check-e2e-logs.py simplebackups-e2e --strict` →
`logs=16 ok=16 missing-exit=0 bad-exit=0` (the first main-leg run hit the documented scenario-J
busy-box flake — `ticks=0` after relaunch — and a clean re-run passed every scenario).

### Package 0.2.11

| | |
|---|---|
| version | **0.2.11** (`horizon-mod.toml` + `Cargo.toml`) |
| bytes | **70,343 B** |
| sha256 | `47ab00b7aa1ee0dd7966fb825a1c1c8016c4d9b2dfd65b4f9ae70a1471da3914` |
| entries | **10** — `horizon-mod.toml`, `Cargo.toml`, `LICENSE`, `LICENSE-NOTICE.md`, `README.md`, `src/{lib,runtime,support,zip}.rs`, `uses.json`; **no `Cargo.lock`** |
| reproducible | two builds, each after `rm -f Cargo.lock`: identical sha and `cmp` (`sdk-cmp.log`) |
| `check` | `simplebackups 0.2.11 (native-mod, format native-1/0a875da…/e59da90…, for 26.3)`, 70,343 B, exit 0 (`sdk-check.log`) |
| `fits --build 0a875da91dbb0fdcb7ba8c42c11341aa229af51a` | `0a875da  made for` (`sdk-fits.log`) |
| `fits --build 0b14f8acfd97fa2c750e42795249dcc981178ca4` | `0b14f8a  fits` — the real gate (`sdk-fits-other.log`) |
| tests | **84 passed, 0 failed** (45 `runtime`, 14 `support`, 12 `zip`) |
| `verify-sdk-package.py simplebackups` | `1/1`, `package matches the tree`, R9 clean (`sdk-verify-package.log`) |

**Remote build path.** Every cargo ladder step forwarded through
`ports/tools/build-slot.sh run --job W1-simplebackups --port ports/simplebackups --step …` to host
**AdLf-PC** (`build-slot: forwarding … to the remote runner`); `BUILD_LOCAL=1` was never set. The
launcher native build and every game leg stayed local (`FORWARDED=1`), because the second box has no
`horizon-launcher` or Nightly versions cache. All e2e scratch dirs came from `E2E_TMP` under
`~/.cache/horizon-e2e/scratch` — never `/tmp`.

### Changelog (0.2.11)

> **0.2.11 — an adoption binds to the archive digest, re-checked before deletion.** Review
> follow-up on PR #1578: filename and size cannot tell a replacement archive from the one an
> operator adopted, so `/simplebackups adopt <file>` now records the SHA-256 beside the name and
> size (`simplebackups-adopted.txt` gains a third column), retention stops treating a replaced file
> as adopted, and both deletion passes re-check the digest immediately before `remove_file`. An
> older two-column journal adopts nothing until re-adopted — a safe default, never a silent
> authorization. 84 tests, including the standard SHA-256 vectors and a support-level
> adopt→replace→survive→re-adopt→prune cycle. Scenario G's new leg proves it in game and
> `SELFTEST=adopt-digest` inverts the survival assertion.

## Next slice

No implementable family remains in this port's gap list at pin `0a875da`. The three survivors are
all measured blocks or out-of-scope: **disk reservation** — `blocked(no-disk-space-api)`, std and
the pinned `horizon-mod-api` have no volume-space query (a std-only `statvfs` would be a large new
subsystem, deliberately not attempted for a review fix; gate log
`../_shared/evidence/simplebackups/reserve-gate-0a875da.log`); **sync-copy** —
`blocked(no-sync-observable)`, the `ctl chat` reply returns in 44 ms before any copy starts, so the
two thread placements cannot be distinguished in one round trip (gate log
`../_shared/evidence/simplebackups/sync-copy-gate-0a875da.log`); **regex ignores** — `not ported`
under R9 (a regex needs a registry dependency). **Cherished Worlds / mc2discord** need mods that are
not in the registry, so they fail slice-selection rule 4. Re-check the two `blocked` gates at every
new pin: the unblock condition is a host free-space method for one and a synchronous command reply
or a thread-placement marker for the other. If a new pin supplies either, the next slice is its
re-verification (replace the fallback, re-run the ladder and the differential, update the record).

