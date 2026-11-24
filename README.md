# Simple Backups for Horizon

A port of [Simple Backups](https://github.com/ChaoticTrials/SimpleBackups) 1.21-4.0.30 (Apache-2.0)
for Horizon 26.3: scheduled, on-disk backups of the server's world, with retention.

This is one flat SDK package (`api = "0.2"`): `horizon-mod.toml` is its identity, `src/lib.rs` the
one `impl mod_api::Hooks`, `src/runtime.rs` the pure backup policy with its tests, `src/support.rs`
the game glue and `src/zip.rs` the archive writer. The launcher compiles it against the game build
`horizon-mod.toml` pins; nothing in `.horizon/` is part of the package.

## What it does

- **Schedule:** `interval_ticks > 0` counts server ticks; otherwise `interval_minutes` of wall clock
  (0 disables the schedule). Due checks run at the end of a running server tick. In tick mode the
  progress toward the next backup is persisted in `simplebackups-schedule.txt` beside the world
  (written on every save and on a graceful close, with a staged rename) and restored when the world
  opens, mirroring upstream's `lastSaved` in its `BackupData` SavedData — a restart resumes the
  interval instead of restarting it. A missing or malformed file starts a fresh interval.
  `/simplebackups status` reports `ticks=<progress>`.
- **Save:** the open levels are asked for a save (`LevelRef::request_save`), whose save runs at the
  start of the next tick; the copy starts when the save cycle reports back (`world_saved`).
- **Backup:** a worker thread walks the world folder into `<world>/backups/<world>_<UTC>.zip.part`,
  publishes it by renaming, and skips `session.lock`, `.part`/`.tmp`, symlinks and the backups
  folder. Entries are `<world>/<relative path>`.
- **Retention:** after a successful publish, the oldest archives this port wrote are pruned until
  `keep` remain (`keep` is at least 1). An archive counts as this port's when its own
  `simplebackups.json` entry names this port and this world. An archive written before that marker
  existed (0.2.0–0.2.2) has no identity until an operator runs `/simplebackups adopt <file>`: the
  `<world>/…` + `<world>/level.dat` shape is exactly what a hand-made export looks like, so the
  shape alone is never enough. A ZIP that merely matches the `<world>_*.zip` name prefix is never
  deleted. Foreign files and `.part` files are untouched.
- **Disk cap:** after the same publish, the backup folder is measured and this world's oldest
  **identified** archives are deleted until it is at most `max_disk_size` (default `"25 GB"`, binary
  units, upstream's `maxDiskSize`). `/simplebackups status` reports
  `disk=<size> cap=<size> over=<bool> adopted=<n>`; foreign files count toward the folder size but
  are never deleted. Same-second backups (`-1`, `-2`, … `-10`) are ordered by their numeric
  generation, and a new backup is always allocated **above the highest suffix present**, so a pruned
  suffix is never reused. The archive a publish just wrote is never one of either pass's removal
  candidates.
- **A save that overlaps a copy:** the finished copy is thrown away and redone (`retry_on_save`,
  up to two retries) instead of publishing a possibly half-replaced region file. When retries are
  off, it is published and `/simplebackups status` reports `stale=true`; **both retention passes are
  then skipped**, so the previous confirmed restore point is never deleted in favour of a copy a
  save may have caught half-written (the folder may stay over the cap until the next good backup).
- **Commands** (level 2): `/simplebackups backup [quiet]`, `/simplebackups status`,
  `/simplebackups list`, `/simplebackups adopt <file>`, and the pause pair
  `/simplebackups pause` / `/simplebackups unpause` (also accepted as
  `/simplebackups backup pause|unpause`, upstream's spelling). A manual backup ignores both
  `enabled` and `paused`. `adopt` records a marker-less archive (by name and size, in
  `simplebackups-adopted.txt` in the backup folder) as this world's, which is the only way it
  becomes deletable.
- **Pause:** `pause` suppresses only the schedule, exactly like upstream's `shouldRunBackup`
  checking `BackupData.isPaused()`; a manual `backup` still runs. The flag is persisted in
  `simplebackups-paused.txt` beside the world and reloaded when the world opens, mirroring
  upstream's `paused` field in its `BackupData` SavedData. `/simplebackups status` reports
  `paused=<bool>`. The client-side red HUD banner the jar sends over its `Pause` packet is not
  ported (a `client`/UI half, outside this server-only logic port).
- **Config** `config/simplebackups.toml`: `enabled`, `interval_minutes`, `interval_ticks`, `keep`,
  `max_disk_size`, `compression_level`, `save_before_backup`, `retry_on_save`, `broadcast`,
  `backups_dir`, `ignored_paths`, `ignored_files`. A missing file is written with the defaults.

## Notes on this port

- Entries are **stored** (no compression): an SDK mod may not add a registry crate, so the archive
  is written by `src/zip.rs` with `std` alone. Backups are therefore the size of the world rather
  than deflated; `compression_level` is still parsed and reported.
- Backups live in `<world>/backups` and are named in UTC (upstream used the working directory and
  local time).
- The port is published as **Apache-2.0**, the same as upstream; see `LICENSE-NOTICE.md` and
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
