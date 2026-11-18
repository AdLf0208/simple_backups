//! Simple Backups for Horizon: scheduled world backups with retention.
//!
//! Simple Backups (see `notes.md`) backs a server's world up on a schedule: a save, then a
//! worker thread that walks the world folder into a `.zip.part`, publishes it as a `.zip`, prunes
//! the oldest ones down to `keep`, and reports the result. The rules — schedule arithmetic, names,
//! retention, skip rules — are [`runtime`]'s, carried byte-identical from the retired `.hnmod`
//! bundle. [`support`] is the glue: the config file, the world folder, the save request, the
//! worker and the commands; [`zip`] writes the archive with nothing but `std`, because an SDK mod
//! may not add the `zip` crate the bundle used.
//!
//! The moments this mod listens at:
//!
//! - `server_about_to_start`: read `config/simplebackups.toml`, writing the defaults.
//! - `world_lifecycle` / `server_stopping`: forget the world / stop scheduling.
//! - `server_tick`: the end of a running tick polls the worker and starts a due or queued backup.
//! - `world_saved`: a save cycle completed — count it, and start the copy that was waiting.
//! - `register_commands` / `run_mod_command`: `/simplebackups backup|status|list` (level 2).
//!
//! The jar carries no blocks, items, recipes, tags or models, so nothing is carried: the package
//! has no `assets/` or `data/`.
pub mod runtime;
pub mod support;
pub mod zip;

use mod_api::{
    self, Hooks, RegisterCommandsEvent, RunModCommandEvent, ServerAboutToStartEvent, ServerHost,
    ServerStoppingEvent, ServerTickEvent, ServerTickPhase, WorldLifecycleEvent, WorldSavedEvent,
};

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    /// Before the server opens anything: read or create `config/simplebackups.toml`.
    fn server_about_to_start(
        &self,
        host: &mut dyn ServerHost,
        _event: &mut ServerAboutToStartEvent,
    ) {
        support::load_config(&*host);
    }

    /// A world closed (finally, or with a failed save): forget its folder. There is no "opened"
    /// phase at this pin, so the open world is taken at its first running tick.
    fn world_lifecycle(&self, _host: &mut dyn ServerHost, event: &mut WorldLifecycleEvent<'_>) {
        support::on_world_lifecycle(event.phase);
    }

    /// The end of a running tick: poll the worker, and start a due or queued backup. A frozen
    /// step does not advance the world, so it is ignored.
    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        let world_running = match event.phase {
            ServerTickPhase::ServerTickEnd { world_running } => world_running,
            // The phase list is not exhaustive, and `ServerTickStart` is not the end of a tick.
            _ => return,
        };
        support::on_server_tick(host, world_running);
    }

    /// A save cycle completed (`wait` says its writes were flushed): count it, and start the copy
    /// that was waiting for it.
    fn world_saved(&self, host: &mut dyn ServerHost, event: &mut WorldSavedEvent) {
        support::on_world_saved(host, event.wait);
    }

    /// The server is stopping: no new backups.
    fn server_stopping(&self, _host: &mut dyn ServerHost, _event: &mut ServerStoppingEvent) {
        support::on_server_stopping();
    }

    /// Declare `/simplebackups backup|status|list` at permission level 2.
    fn register_commands(&self, _host: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        support::register_commands(event.commands);
    }

    /// Run them. The dispatcher has already parsed the usage, so `quiet` is a typed boolean.
    fn run_mod_command(
        &self,
        host: &mut dyn ServerHost,
        event: &mut RunModCommandEvent<'_>,
    ) -> mod_api::Flow {
        support::run_command(host, event)
    }
}
