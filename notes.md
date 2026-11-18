# Simple Backups — port notes

Ported from **Simple Backups 1.21-4.0.30** (`ChaoticTrials/SimpleBackups`,
**Apache-2.0**) for Minecraft Java 1.21.1 NeoForge. Server-only.

Status: **implemented; verified in game at 0.1.0** — 15 runtime tests, `stage → check → test →
build` green, and the backup scenarios pass (see below). **Republished as the SDK package
0.2.0** on 2026-11-18: 23 tests, clippy/fmt clean, `check`/`fits` green, artifact reproducible;
**not yet re-run in game** (no launcher here) — see *Migration to the SDK format*.

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
- Retention prunes after success (upstream deletes before creating), and only
  `<world>_*.zip` (upstream counts every file); both are deliberate safety
  changes.
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
4. Still deferred, unchanged: incremental/chains/merge, pause/unpause, `latest.log` capture, a disk
   cap, the empty-server policy, Cherished Worlds/mc2discord, regex ignores, a persisted schedule,
   and a synchronous-copy mode.
5. `/simplebackups status` is the only route for a config warning; there is no log file.
6. `side = "both"`, so the mod is installed on a client too; that is required for single player and
   a LAN host (their integrated server belongs to the client installation).

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices, rust-port

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
