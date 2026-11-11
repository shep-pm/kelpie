//! What a worker runs on, resolved from the settings
//!
//! [`plan`] is for workers what [`pm::plan`](crate::pm::plan) is for the PM: it
//! reads the project's settings and the worker agent on every start, so a change
//! made on the settings page applies to the next worker.

use std::time::Duration;

use kelpie_engine::agents::{Agents, Binding};
use kelpie_engine::sandbox::Reads;
use kelpie_engine::settings::{ConfigDir, Settings};

/// What a worker is started on, resolved from the settings
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerPlan {
    /// How many of the project's workers run at once (`worker-slots`)
    pub slots: usize,

    /// How long a worker's turn may run (`worker-turn-timeout`)
    pub turn_timeout: Duration,

    /// Domains the sandbox lets a worker reach besides its model's host and
    /// GitHub (`network-allowlist`)
    pub network_allowlist: Vec<String>,

    /// How far a worker reads (`read-anywhere`)
    pub reads: Reads,

    /// Whether the harness reads the person's own global folder (`harness-global-folder`)
    pub harness_global_folder: bool,

    /// What a worker runs as: `kelpie:worker`, then the global `worker.md`, then
    /// the project's own setting
    pub binding: Binding,
}

/// What a worker runs on for the project `project_id`, from the files in `config`
///
/// # Errors
///
/// A settings file or the worker's agent file cannot be used. The text names
/// the file and what is wrong with it.
pub fn plan(config: &ConfigDir, project_id: &str) -> Result<WorkerPlan, String> {
    let settings = Settings::load(config, project_id).map_err(|error| error.to_string())?;
    let agents = Agents::load(config).map_err(|error| error.to_string())?;
    let binding = agents
        .worker_binding(&settings)
        .map_err(|error| error.to_string())?;
    Ok(WorkerPlan {
        slots: settings.worker_slots.value.places(),
        turn_timeout: settings.worker_turn_timeout.value.duration(),
        network_allowlist: settings.network_allowlist.value,
        reads: Reads::from_setting(settings.read_anywhere.value),
        harness_global_folder: settings.harness_global_folder.value,
        binding,
    })
}
