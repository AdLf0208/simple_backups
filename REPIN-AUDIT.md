# Simple Backups — re-pin audit: Horizon `83ad384f`

Read-only source audit. **No port code was changed and nothing was built.**

| | |
|---|---|
| New Horizon | `83ad384f48d0742746d518737511882680aab5b7` — 427 hooks / 159 helpers, at `ports/_shared/pins/horizon-83ad384f` |
| New mixins | `e8cd0aa38cffef49cb2e088ef716baa107148c02`, at `ports/_shared/pins/horizon-mixins-e8cd0aa` |
| Old pin (baseline) | `c9d474d` — 108 hooks, **0 helpers** |
| Port state | slice 1: 15 runtime tests, 6 injections (all v1, all still present) |

Classification: **usable** = a helper a plan can call, a writable/cancellable hook output, or a
content field the loader reads; **hook-only** = listen, cannot write or deny; **absent** = no
symbol. Paths are relative to the pin root.

## Summary

| | |
|---|---|
| Deferred items in `notes.md` | **13** (5 pin-relevant, 6 pure-Rust, 2 external-mod compat) |
| Newly realizable at `83ad384f` | **3** (save-cycle detection to **detect and retry** a raced copy; a real `save-off`/`save-on` window on dedicated servers; `instance_dir` for `latest.log`) |
| Realizable already, effort only | 6 (persisted schedule, pause/unpause, disk cap, regex ignores, sync-copy mode, local-time naming/location) |
| Still blocked | **3** (a true autosave veto on integrated worlds, Cherished Worlds, mc2discord) + the stale `native_world_dir` caveat |
| Highest-value next step | **`world_saved`-driven retry of a copy that overlapped a save** — works on integrated and dedicated |

## 1. Deferred items, one row each

| # | Deferred (`notes.md:56-75`, `:110-126`) | Pin symbol / file:line | Class | Verdict |
|---:|---|---|---|---|
| 1 | **Consistency window**: no save-off/autosave veto, so the worker can copy a region file while an autosave rewrites it | **no veto hook** (no `server_autosave`/`save_off` in `native-hooks.json`). New: `world_saved(wait)` hook `native_hooks.rs:5915` (catalog `native-hooks.json:4471`, v1, `listen`), fired by every save at `game.rs:15625`; `native_run_command` `native_server.rs:479`; `/save-off`/`/save-on`/`/save-all` `commands.rs:1118-1119` → `commands/profiles.rs:47,66`, gated on a dedicated server (`dedicated_lists`, `profiles.rs:18-28`) | veto: **absent**; detection: **usable (new)**; dedicated veto: **usable** (commands + `dedicated::set_auto_save`, `dedicated.rs:796`) | **newly realizable: detect + retry everywhere; real veto on dedicated servers only** |
| 2 | Schedule not persisted across restarts | `world_lifecycle` v1 (already bound) + a small world file; pure runtime code | usable | realizable (port code) |
| 3 | Empty-server policy | `native_server_players` `native_server.rs:204-226` (guests only — see §3) | usable (caveat: the host) | realizable (host rule documented) |
| 4 | Disk-size cap | `fs` walk of the backups dir; pure runtime code | usable | realizable (port code) |
| 5 | Pause/unpause | `register_commands`/`perform_command` v1 (already bound) + a flag | usable | realizable (port code) |
| 6 | `latest.log` capture | `native_instance_dir` `native_hooks.rs:9183` (**new**) → `logs/latest.log`, worker adds it | usable (**new**) | realizable |
| 7 | Incremental backups / chains / merge | pure runtime code; no pin dependency (upstream `1.21.1` chains; the port may design its own) | usable | realizable (effort) |
| 8 | Cherished Worlds compat | external mod's API, not in Horizon | **absent** | **still blocked** |
| 9 | mc2discord compat | external mod's API, not in Horizon | **absent** | **still blocked** |
| 10 | Regex ignores | pure runtime code (no `regex` dep: a hand-rolled glob or a `regex` dependency) | usable | realizable |
| 11 | Sync-copy mode (tick stall) | pure runtime code; `save_game()` already on the server thread (`game.rs:15586`) | usable | realizable |
| 12 | UTC naming / `<world>/backups` location (deviations) | pure runtime code | usable | realizable (cosmetic, keep UTC by default) |
| 13 | `native_world_dir` not cleared after a close | `native_server.rs:492-496` checks `world_root` only; `world_root` is set at `game.rs:12451` and never cleared | **absent** (pin bug) | **still blocked**; the port's "trust only between open and close" guard stays |

## 2. The consistency window in detail

The pin still has **no way to pause autosave from a hook**. The autosave runs from a countdown
(`ticks_until_autosave`, `game.rs:2719-2720`, decremented and reset at `:7717-7724`,
`next_autosave_interval` `:15592`, 6000 ticks at 20 tps per `autosave_interval` `:17297`), and
nothing in the hook catalog can cancel or postpone it.

What is new:

- **`world_saved(wait)`** (`native_hooks.rs:5915`, new function **and** new catalog entry).
  `save_game_with` fires it for every save, including the port's own `save_game()`
  (`game.rs:15611-15626`) and every autosave. A port can therefore *know* an autosave ran while
  its zip was being written, which the old pin could not.
- **`/save-off`, `/save-on`, `/save-all` now exist** (`commands.rs:1118-1119`,
  `commands/profiles.rs:47-83`) and `native_run_command("save-off")` (`native_server.rs:479`)
  can run them — but only on a **dedicated** server: `cmd_auto_save`/`cmd_save_all` call
  `dedicated_lists()` (`profiles.rs:18-28`) and return `command.unknown.command` on an
  integrated server. A dedicated server keeps a `lan` host (e.g. `dedicated.rs:1493` restores
  it each tick), so `native_run_command` normally passes its `lan.is_none()` guard — worth one
  smoke check on a real dedicated run; it still refuses inside a player hook
  (`if self.lan_acting.is_some() || self.lan.is_none()`) and works from `server_tick`.
- The port's own save is still the best flush: `fn save_game` (`game.rs:15586`) →
  `save_game_with(true)` waits for chunk writes, and on a dedicated server it now also records
  the management protocol's `saving`/`saved` events (`game.rs:15612-15626`).

So there are two honest modes:

1. **Detect and retry (works everywhere).** Keep a `save_seq` counter fed by a `world_saved`
   injection; when a backup starts, snapshot the sequence and the region-file mtimes; if the
   sequence moves while `in_flight` (an autosave landed mid-copy), do not publish the `.part`:
   retry after the next quiet tick, up to a small limit, and report the retry. This removes the
   race instead of hiding it, without touching the game's save schedule.
2. **Real veto (dedicated only).** Around the copy: `native_run_command("save-off")` →
   `save_game()` → worker → on completion `native_run_command("save-on")`. Must be paired with
   a fallback that always re-enables (`server_stopping`, worker error, world close), or a crash
   leaves autosave off forever. `dedicated::set_auto_save` (`dedicated.rs:796`) is the same
   switch reached without the command; using the command is safer against future signature
   changes.

## 3. Still blocked / caveats

1. **Autosave veto on integrated worlds.** No command and no hook path: `/save-off` is
   dedicated-only, and `ticks_until_autosave` is a private field of `Game` with no setter. Mode 1
   is the only option there.
2. **Cherished Worlds / mc2discord.** Different mods; nothing to port.
3. **Empty-server policy vs the host.** `native_server_players` lists `lan.hub.guests` only
   (`native_server.rs:204-226`), so an integrated world with the host playing looks "empty". The
   policy must treat the open world's own player as a player (`game.player` is `Some`), and
   guests as extra; document it.
4. **`native_world_dir` is still stale after a close.** `world_root` is set at `game.rs:12451`
   and never cleared, while the doc says "None when no world is open" (`native_server.rs:492-496`).
   The port's existing guard (only between `WorldOpenReady` and `WorldCloseFinal`) stays.

## 4. Next step (highest value): save-cycle retry

1. Migrate first (shared recipe): `compatibility.json`, manifest + mixin `horizon_revision` to
   the new pins; `tools/horizon.py` test target `--lib`, ctl package `horizon-client-lib`;
   ladder under `CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2`.
2. New injection, v1: `hook = "world_saved"`, `helper = "saved.rs"`, bindings `game` and `wait`
   (`wait: bool`); the helper calls `simplebackups_state::on_world_saved(game, wait)` which only
   bumps a `save_seq: u64` in `Core` (cheap, runs on every save).
3. `support.rs`: in `start_backup`, record `save_seq` and the newest mtime under `world/region`
   just after `save_game()`; keep both with the in-flight job. Move the final `fs::rename` out of
   `run_backup` (`support.rs:386`) into `on_server_tick`: when the worker reports success and
   `save_seq` moved during the copy, delete the `.part`, re-queue the backup once (retry cap 1),
   and only rename when the sequence is unchanged. Expose `retries` in `/simplebackups status`.
4. e2e proof (fail-capable, integrated world — no dedicated server needed): force an autosave
   during a copy by shortening the interval with `/tick sprint` (the interval follows the measured
   tick rate: `next_autosave_interval`, `game.rs:15592-15600`, minimum 100 ticks). Start a backup
   of a world big enough to outlast the next save, then assert
   `/simplebackups status` reports `retries=1` and the published zip is valid and complete (entry
   count equal to a fresh run's). The differential: the same scenario with the retry disabled
   (`config` switch, or `NEGATIVE=1`) must report `retries=0` while the log shows the autosave
   landing mid-copy — so the assertion is proven non-vacuous. Do not try to assert a corrupted
   region inside the first zip: that race is timing-dependent, the retry counter is not.
5. Optional second job: the dedicated-only veto (§2 mode 2) with its own e2e on a dedicated
   server; do not mix it into the same brief, because the failure mode (autosave left off) needs
   its own teardown test.

## 5. Migration notes for this port

- All six hooks are still v1 with the same names and parameter sets: `server_about_to_start`,
  `world_lifecycle` (`phase`), `server_tick` (`phase`), `server_stopping`,
  `register_commands`, `perform_command` (`player`, `command`, `ran`).
- `Game::save_game` still exists and is still private to module `game` (`game.rs:15586`), so the
  injected `simplebackups_state` module keeps calling it; `save_game(wait = true)` is the port's
  flush.
- No content change: the `.hmod` is unaffected by the re-pin.
