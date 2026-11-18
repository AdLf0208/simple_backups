# Simple Backups for Horizon

A port of [Simple Backups](https://github.com/ChaoticTrials/SimpleBackups) 1.21-4.0.30 (Apache-2.0)
for Horizon 26.3: scheduled, on-disk backups of the server's world, with retention.

This is one flat SDK package (`api = "0.2"`): `horizon-mod.toml` is its identity, `src/lib.rs` the
one `impl mod_api::Hooks`, `src/runtime.rs` the pure backup policy with its tests, `src/support.rs`
the game glue and `src/zip.rs` the archive writer. The launcher compiles it against the game build
`horizon-mod.toml` pins; nothing in `.horizon/` is part of the package.

## What it does

- **Schedule:** `interval_ticks > 0` counts server ticks; otherwise `interval_minutes` of wall clock
  (0 disables the schedule). Due checks run at the end of a running server tick.
- **Save:** the open levels are asked for a save (`LevelRef::request_save`), whose save runs at the
  start of the next tick; the copy starts when the save cycle reports back (`world_saved`).
- **Backup:** a worker thread walks the world folder into `<world>/backups/<world>_<UTC>.zip.part`,
  publishes it by renaming, and skips `session.lock`, `.part`/`.tmp`, symlinks and the backups
  folder. Entries are `<world>/<relative path>`.
- **Retention:** after a successful publish, only `<world>_*.zip` files are pruned oldest-first
  until `keep` remain (`keep` is at least 1). Foreign files and `.part` files are untouched.
- **A save that overlaps a copy:** the finished copy is thrown away and redone (`retry_on_save`,
  up to two retries) instead of publishing a possibly half-replaced region file; when retries are
  off, it is published and `/simplebackups status` reports `stale=true`.
- **Commands** (level 2): `/simplebackups backup [quiet]`, `/simplebackups status`,
  `/simplebackups list`. A manual backup ignores `enabled`.
- **Config** `config/simplebackups.toml`: `enabled`, `interval_minutes`, `interval_ticks`, `keep`,
  `compression_level`, `save_before_backup`, `retry_on_save`, `broadcast`, `backups_dir`,
  `ignored_paths`, `ignored_files`. A missing file is written with the defaults.

## Notes on this port

- Entries are **stored** (no compression): an SDK mod may not add a registry crate, so the archive
  is written by `src/zip.rs` with `std` alone. Backups are therefore the size of the world rather
  than deflated; `compression_level` is still parsed and reported.
- Backups live in `<world>/backups` and are named in UTC (upstream used the working directory and
  local time).
- The port is **local-only** until the maintainer approves publishing; see `LICENSE-NOTICE.md` and
  `notes.md` for the licence provenance, the gaps and the verification ladder.

## Building and checking

The package is packed by the launcher's tooling; it is not compiled here:

```sh
cargo test --offline                                   # the pure rules and the archive writer
cargo clippy --all-targets -- -D warnings
rm -f Cargo.lock && cargo horizon-mod build            # target/horizon-mod/simplebackups.hmod
cargo horizon-mod check simplebackups.hmod
cargo horizon-mod fits simplebackups.hmod --build nightly
```

The native compile of the mod itself is the launcher's:
`horizon-launcher native-build . --instance <instance> --trust-native-code`.
