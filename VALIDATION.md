# Validation

> **Historical — the retired `.hnmod` format (0.1.0).** Everything below was recorded against the
> pre-SDK bundle (`compatibility.json`, `tools/horizon.py`, the mixin plan), which no longer loads on
> launcher ≥ 0.2.70.

## Current: SDK package 0.2.10 (2026-11-24, pin `0a875da`)

The ladder and its logs live in [`notes.md`](notes.md) under *Slice 9 (0.2.9)* and
`../_shared/evidence/simplebackups/`. Every cargo step was forwarded through
`ports/tools/build-slot.sh run --job W1-simplebackups --port ports/simplebackups --step …` to
host **AdLf-PC** (`build-slot: forwarding … to the remote runner`); `BUILD_LOCAL=1` was not set.
The launcher build and the game legs stayed local (`FORWARDED=1`) because the second box has no
`horizon-launcher` or Nightly versions cache (only the repaired SDK CLI and the 26.3 resources).

**Slice 10 (2026-11-24) — `blocked(no-disk-space-api)`; nothing re-validated because nothing
changed.** Upstream's per-file reservation (`Files.getFileStore(output).getUsableSpace() -
file_size - 128 MiB`, `BackupThread.java:56,374,395-398`) cannot be expressed at this pin: std has
no free-space query and the pinned `horizon-mod-api` surface has none (the only `space_for` is item
storage, `available_space` is a GUI field), and R9 forbids adding a `statvfs` crate. No code,
config, version or package changed; the delivered artifact is still 0.2.9 (65,996 B, sha256
`53a767bb1c2a55915391ea69630fd4f36bad5a30ec8727bac820e2ad8bb31606`). Re-checks after the blocked
slice: `check-e2e-logs.py simplebackups-e2e --strict` → `logs=14 ok=14`, `verify-sdk-package.py
simplebackups` → `1/1`, `check-skills.py --notes ports/simplebackups` → PASS. Gate evidence:
`../_shared/evidence/simplebackups/reserve-gate-0a875da.log`.

**Slice 11 (2026-11-24) — `blocked(no-sync-observable)`; nothing re-validated because nothing
changed.** `copy_on_server_thread` would move the existing `run_backup` call from the worker into
the save-complete moment, but the harness cannot see the two placements in one `ctl` round trip:
the 0.2.9 game answers `ctl chat "/simplebackups backup"` in **0.044 s** (`{"ok":true}`), an
immediate `list` (**0.379 s**) names **0** archives and still reads `ticks=42 in_flight=false`, and
a 128 MiB archive appears **10.634 s** after the chat reply. The command only queues; the copy
starts at `ServerTickEnd`/`world_saved`, so no inline placement can make that reply wait for the
archive. No code, config, version or package changed; the delivered artifact is still 0.2.9
(65,996 B, sha256 `53a767bb…31606`). Re-checks after this gate: `fits --build
0a875da91dbb0fdcb7ba8c42c11341aa229af51a` → `0a875da  made for`, `fits --build
0b14f8acfd97fa2c750e42795249dcc981178ca4` → `0b14f8a  fits` (both forwarded through
`build-slot.sh`, `BUILD_LOCAL=1` never set); `verify-sdk-package.py simplebackups` → `1/1`
(`package matches the tree`); `check-e2e-logs.py simplebackups-e2e --strict` → `logs=14 ok=14`;
`check-skills.py --notes ports/simplebackups` → PASS. Gate evidence:
`../_shared/evidence/simplebackups/sync-copy-gate-0a875da.log` (probe
`ports/simplebackups/tools/probe-synccopy.sh`).

**Slice 12 (2026-11-24) — the empty-server interval (`noPlayerBackupTimer`).** One config key,
one pure `empty_period_interval` rule, one branch in the wall-clock due gate, and `timers=<configured>`
on the existing `saves=` status line. Verbatim red (a deliberately wrong stub): 77 passed, 3 failed —
`a_zero_empty_timer_falls_back_to_the_main_interval` (`left: 0`, `right: 120`),
`an_explicit_empty_timer_replaces_the_main_interval` (`left: 120`, `right: 7`),
`the_no_player_timer_key_defaults_to_zero_and_round_trips` (`unknown key: no_player_backup_timer`).
Green 80/0; scenario N parses the newest line for `timers=7`, `SELFTEST=noplayertimer` → `EXIT:1`.
Both interval branches are unit-only: a `ctl launch` always has its owner online (`players=1`).

| Check | Result | Log |
|---|---|---|
| `cargo test --offline` | 80 passed, 0 failed (46 `runtime`, 14 `support`, 12 `zip`) | `sdk-test.log` |
| red run before the empty-timer implementation | 77 passed, 3 failed (0 branch, explicit branch, key round-trip) | `sdk-test-red-noplayertimer.log` |
| red run before the log-capture implementation | 73 passed, 4 failed (entry name, path rule, config key, guarded read) | `sdk-test-red-latestlog.log` |
| `cargo clippy --offline --all-targets -- -D warnings` | exit 0, 0 warnings | `sdk-clippy.log` |
| `cargo fmt --check` | clean | `sdk-fmt.log` |
| `rm -f Cargo.lock && cargo horizon-mod build` | `Built ./target/horizon-mod/simplebackups.hmod`, exit 0 | `sdk-build.log` |
| second build + `cmp` | byte-identical, 66,949 B, sha256 `ab6e2312…` | `sdk-cmp.log` |
| `cargo horizon-mod check` | format `native-1/0a875da…/e59da90…`, for 26.3, 66,949 bytes, exit 0 | `sdk-check.log` |
| `fits --build 0a875da91dbb0fdcb7ba8c42c11341aa229af51a` | `0a875da  made for`, exit 0 | `sdk-fits.log` |
| `fits --build 0b14f8acfd97fa2c750e42795249dcc981178ca4` | `0b14f8a  fits`, exit 0 (the real gate) | `sdk-fits-other.log` |
| `verify-sdk-package.py simplebackups` | 1/1, `package matches the tree`, R9 clean | `sdk-verify-package.log` |
| `unzip -l` | 10 entries, no `Cargo.lock` | `sdk-entries.log` |
| launcher `native-build` | `Compiling simplebackups (1 of 1)`, tag `native-0a875da-b8e99fd6e39a` | `../simplebackups-e2e/build-record.txt` |
| in game | `PASS` + `EXIT:0` (scenarios A–N, M-off incl. L/L2); control `EXIT:1`; `SELFTEST=1` `EXIT:1`; `SELFTEST=pause` `EXIT:1`; `SELFTEST=ticks` `EXIT:1`; `SELFTEST=chain` `EXIT:1`; `SELFTEST=noplayer` `EXIT:1`; `SELFTEST=latestlog` `EXIT:1`; `SELFTEST=noplayertimer` `EXIT:1`; `STALE_SELFTEST=1` `EXIT:1`; the two older negatives `EXIT:1` | `../simplebackups-e2e/e2e-launcher*.log` |
| e2e log gate | `logs=15 ok=15 missing-exit=0 bad-exit=0` (`--strict`) | — |

The in-game playerless window was **not** producible: `status` reported `players=1` on every 0.2.8
and 0.2.9 run (the harness owner is a placed player and `ctl disconnect` needs a remote server). The
empty branch is proven by the five red-first unit tests; scenarios L/L2 prove the online branch and
record the observed count. No chat command was passed off as an empty server.

The 0.2.9 log-capture path was **measured** before implementation: the headless launcher writes
`$scratch/game/logs/latest.log` while `host.instance_dir()` is `$scratch` and `HORIZON_HOME` is
`$scratch/game` (production sets them equal). The glue derives candidates from both, never from an
absolute path; scenario M asserts an exact line from that file is inside `backup.log`, and the
key-off scene asserts no such entry.

The earlier packages were validated the same way at pin `0a875da`: 0.2.7 in *Slice 7* of
`notes.md`, 0.2.6 in *Slice 6*, 0.2.5 in *Slice 5*, 0.2.4 in *Slice 4*, 0.2.3 in *Slice 3*, 0.2.2
in *Slice 2*; 0.2.1 was at pin `0b14f8a` (its e2e logs are in
`../_shared/evidence/simplebackups-e2e/retired-0.2.1/`). The current `build-record.txt` is the
0.2.10 build; 0.2.8/0.2.9 are in *Slice 8* / *Slice 9* of `notes.md`.

## Historical — 0.1.0 `.hnmod`

Validated on Linux with Rust 1.95.0 on 2026-10-30, using the exact revisions in
`compatibility.json` and the real bundle committed here.

| Check | Result |
|---|---|
| `cargo fmt --all --check` | Pass |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Pass |
| `cargo test --workspace --locked --offline` | Runtime unit test passes |
| `python3 tools/test_template.py` | Actual bundle compiles in the tiny fixture; original method calls/result, shared runtime, AST audit, unchanged bytes/mtimes and incompatible revision/target rejection pass |
| `python3 tools/test_stage.py` | Complete pinned Horizon export stages deterministically; original checkout unchanged; exact HEAD/module audit passes |
| `python3 tools/horizon.py check` | Actual `horizon-client` binary checks successfully with `--no-default-features` |
| `python3 tools/horizon.py test` | The runtime's embedded test passes inside the actual Horizon game test binary |

The native check reported 69.55 seconds in Cargo, and the native test passed.
Both reused an existing external Cargo target directory with cached
Horizon dependencies. These are compilation/test checks, not a fresh-build
benchmark. Staging the full export took about 24 seconds per command on this
machine; a repeated application preserved the stage.

No game was launched and no Minecraft resources or player data were read or
changed. Interactive logging/gameplay remains a manual check with the compatible
prepared resources. The source audit verifies only the declared HEAD statement
and support module differ from pristine Rust ASTs; it does not claim compatibility
with other Horizon revisions or arbitrary third-party mods.

CI reproduces the formatting, clippy, standalone runtime and tiny-fixture tests.
The full native checks additionally need a Horizon Git history containing the
pin and its cached dependencies; use the commands in README.md to reproduce them.
