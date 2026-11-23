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
| stale copy skips retention | tested | scenario F: `stale=true` with `keep=1` and a `1 B` cap leaves both archives and `over=true`; `STALE_SELFTEST=1` asserts the pre-fix defect and exits 1 |
| same-second generations | tested | `runtime::tests::{same_second_backups_sort_by_generation_not_as_text,the_cap_also_orders_same_second_generations_numerically,a_pruned_suffix_is_never_reused_for_a_new_backup}`; scenario H: a backup in a second that already has `-41`/`-42` becomes `-43` and both `-42`/`-43` survive `keep=2`; H2: the cap keeps only `-43`. The 0.2.3 package writes the base name and deletes it in the same pass (`e2e-launcher-same-second-negative.log`, `EXIT:1`) |
| malformed archive name | tested | `runtime::tests::a_name_with_a_multibyte_character_at_the_stamp_boundary_is_refused`: a two-byte character at byte 19 is refused, not sliced (the pre-fix `split_at(19)` panicked) |
| commands `backup` / `status` / `list` / `adopt` | tested | `ctl chat "/simplebackups backup"` and `status` in every scenario, `list` in scenario D: the reply names every archive in the folder, and after a `1 B` cap it no longer names the deleted one; `adopt` in scenario G (confirmed reply, `adopted=1`, and a refusal for a name that is not this world's) |
| save-overlap retry (`retry_on_save`) | tested | unit test `should_retry`; scenario F forces a real overlap with `ctl save` during a 256 MiB copy (`stale=true`, `saves=6`); scenario C's pause/resume still prints `SKIP` (that path cannot make a save) |
| `pause` / `unpause` | not ported | gap (commands are `backup`, `status`, `list` only) |
| empty-server policy (`noPlayerBackups`) | not ported | gap; needs a player-presence API and a way to empty the server headless |
| `latest.log` → `backup.log` | not ported | gap (needs a path outside the world folder) |
| disk reservation before a copy | not ported | gap (upstream's `NotEnoughDiskSpaceException`) |
| partial / chains / merge, regex ignores, persisted schedule | not ported | gaps: the pin has no game-time API for tick persistence, and a regex needs a crate |

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
- The schedule is not persisted across restarts; it starts at world open.
  Empty-server policy, disk cap, pause/unpause, `latest.log` capture,
  incremental/chains/merge, Cherished Worlds/mc2discord and regex ignores are
  deferred.
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
- **Deferred:** incremental/chains/merge, pause/unpause, `latest.log`
  capture, disk-size cap, empty-server policy, Cherished Worlds, mc2discord,
  regex ignores, persisted schedule, sync-copy mode.
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
4. Still deferred, unchanged: incremental/chains/merge, pause/unpause, `latest.log` capture, the
   empty-server policy, Cherished Worlds/mc2discord, regex ignores and a persisted schedule
   (**the disk cap is now done — see *Slice 2 (0.2.2)*, which also adds `max_disk_size`**).
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
