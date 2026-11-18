# Simple Backups — a Horizon port

**Simple Backups** is a Minecraft mod. A simple mod to create scheduled backups. This repository is a **port of it to [Horizon](https://github.com/horizon-rust/horizon)**, the Rust engine for Minecraft Java 26.3. It carries the complete source of the port plus the content files it ships, and installs as a single `.hmod` package.

Upstream: [ChaoticTrials/SimpleBackups](https://github.com/ChaoticTrials/SimpleBackups) — licence **Apache-2.0**.

| | |
|---|---|
| Upstream mod | Simple Backups ([source](https://github.com/ChaoticTrials/SimpleBackups)) |
| Ported to | Horizon 0b14f8a, Minecraft Java 26.3, mod API `0.2` |
| Package | `simplebackups.hmod` (install it with the Horizon launcher) |
| Licence | **Apache-2.0** — the upstream mod's licence; see [Licence and credit](#licence-and-credit) |

## What this port implements

- **Schedule**: `interval_ticks > 0` counts server ticks; otherwise
- **Save**: `Game::save_game()` (flushes chunks, parked levels, `level.dat`,
- **Backup**: a worker thread walks the world folder into
- **Retention**: after a successful rename, only `<world>_*.zip` files are
- **Commands** (level 2): `/simplebackups backup [quiet]`, `status`, `list`.
- **Config** (`config/simplebackups.toml`): `enabled`, `interval_minutes`,
- **Backups are stored, not deflated** (trap 1). They are the size of the world; `compression_level`
- **The save is asynchronous** (trap 3): the copy starts one save-cycle later than 0.1.0's

## Not yet ported

- **Consistency window**: there is no save-off/autosave-veto API at the pin; the
- The schedule is not persisted across restarts; it starts at world open.
- Retention prunes after success (upstream deletes before creating), and only
- Naming is UTC; upstream uses local time. Backup location is
- `native_world_dir` is a pin gap: it is not cleared after a close, so the port
- Licence: upstream is **Apache-2.0** while the template `Cargo.toml`/LICENSE
- **No in-game re-run in this environment.** The e2e needs a game the launcher built with this
- **No deflate**, so a backup cannot be shrunk by `compression_level`; a future SDK that allows a

## Installing

1. Install the [Horizon launcher](https://github.com/horizon-rust/horizon/releases) (0.2.70 or newer).
2. Add `simplebackups.hmod` to the instance's mods.
3. The package is pinned to a specific Horizon build; the launcher checks that pin.

## Licence and credit

Simple Backups was created by its upstream authors; the code, models and assets in the original mod
remain theirs and are used here under **Apache-2.0**. The Rust in this repository is a clean-room
rewrite for Horizon, not a translation of the original source.

- `LICENSE` — the upstream licence text.
- `LICENSE-NOTICE.md` — what this port carries and where it came from.
- The upstream project: https://github.com/ChaoticTrials/SimpleBackups

## Technical details

```
horizon-mod.toml   package manifest (id, version, api, [native] pin)
Cargo.toml         the crate; depends on `horizon-mod-api` from the fetched SDK
src/               lib.rs (the one `impl Hooks`), runtime.rs (pure logic + tests), glue
assets/ data/      the content the package ships
tools/             content generators and the e2e scenario
```

- Build: `cargo horizon-mod build` (packages; it does not compile against the game).
- Test: `cargo test`, `cargo clippy --all-targets -- -D warnings`.
- In-game: `ports/tools/launcher-game.sh run simplebackups` in the campaign tree, or install the
  `.hmod` and play; the harness scenario is `tools/e2e.sh`.

This repository is generated from the campaign's port at `ports/simplebackups/`; the port's own
`notes.md` is the full record (decisions, traps, evidence).
