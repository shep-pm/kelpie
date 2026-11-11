//! The layers a setting's value can come from, and how one is resolved
//!
//! Built-in, then global, then project: the nearest layer that sets a field
//! wins, and [`Layered`] carries which one that was, so a form can show it and
//! offer a reset to the layer below. Which layers may set a field at all is
//! [`Field::layers`], and a field none of them sets falls back to
//! [`BUILT_IN`].

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde_json::{Map, Value};

use super::{BUILT_IN, Field, Resolved, SettingsError};

/// One level a setting's value can come from
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    /// kelpie's own default, in code rather than a file
    BuiltIn,

    /// `settings.json`, the value for every project
    Global,

    /// `projects/<id>/settings.json`, the value for one project
    Project,
}

impl Layer {
    /// What to call the layer where a person reads it
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::BuiltIn => "kelpie",
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

impl fmt::Display for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A value and the layer kelpie took it from
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layered<T> {
    /// The value
    pub value: T,

    /// The layer that set it
    pub from: Layer,
}

/// Which layer file a write goes to
///
/// The built-in layer is in code, so it is never a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerFile<'a> {
    /// `settings.json`
    Global,

    /// `projects/<id>/settings.json`
    Project(&'a str),
}

impl LayerFile<'_> {
    /// The layer this file holds
    #[must_use]
    pub fn layer(self) -> Layer {
        match self {
            Self::Global => Layer::Global,
            Self::Project(_) => Layer::Project,
        }
    }
}

impl Field {
    /// The layers that may set this field
    ///
    /// The PM's and the worker's harness, model and effort are a project's
    /// setting only: their global value is the global `pm` or `worker` agent's
    /// own keys, so one value is never kept in two files. Whose issues a project
    /// picks up, and what it calls Ready, are its own for the same reason: the
    /// ticket source is the project's. How far back Done reaches is the
    /// machine's, or one project's, since how far back to look is a person's.
    #[must_use]
    pub fn layers(self) -> &'static [Layer] {
        const PROJECT: &[Layer] = &[Layer::Project];
        const GLOBAL: &[Layer] = &[Layer::Global];
        const GLOBAL_AND_PROJECT: &[Layer] = &[Layer::Global, Layer::Project];
        match self {
            Self::PmHarness
            | Self::PmModel
            | Self::PmEffort
            | Self::WorkerHarness
            | Self::WorkerModel
            | Self::WorkerEffort
            | Self::WorkerSlots => PROJECT,
            Self::Provider | Self::Host | Self::ReplyStyle | Self::WorkerTurnTimeout => {
                GLOBAL_AND_PROJECT
            }
            Self::NetworkAllowlist
            | Self::ReadAnywhere
            | Self::HarnessGlobalFolder
            | Self::RunWithoutSandbox
            | Self::PickUp
            | Self::ReadyLabel => PROJECT,
            // A lease is the machine's, so its capacity is one value for every
            // project, and so is the appearance of the window they share
            Self::BridgeUpdates | Self::LeaseCapacity | Self::Theme => GLOBAL,
            Self::DoneWindow => GLOBAL_AND_PROJECT,
        }
    }
}

/// One layer's file, read and checked against the schema
pub(crate) struct LayerDoc {
    /// The file it came from, for naming it in an error
    pub(crate) file: PathBuf,

    /// The fields it sets, keyed as the schema keys them
    pub(crate) fields: Map<String, Value>,
}

/// Resolve every field from the two layer files
///
/// # Errors
///
/// [`SettingsError::FileRefused`] when a file's value does not fit the type the
/// schema was written from.
pub(crate) fn resolve(global: &LayerDoc, project: &LayerDoc) -> Result<Resolved, SettingsError> {
    Ok(Resolved {
        pm_harness: Field::PmHarness.resolve(global, project, BUILT_IN.pm_harness)?,
        pm_model: Field::PmModel.resolve(global, project, BUILT_IN.pm_model.map(str::to_string))?,
        pm_effort: Field::PmEffort.resolve(
            global,
            project,
            BUILT_IN.pm_effort.map(str::to_string),
        )?,
        worker_harness: Field::WorkerHarness.resolve(global, project, BUILT_IN.worker_harness)?,
        worker_model: Field::WorkerModel.resolve(
            global,
            project,
            BUILT_IN.worker_model.map(str::to_string),
        )?,
        worker_effort: Field::WorkerEffort.resolve(
            global,
            project,
            BUILT_IN.worker_effort.map(str::to_string),
        )?,
        worker_slots: Field::WorkerSlots.resolve(global, project, BUILT_IN.worker_slots)?,
        worker_turn_timeout: Field::WorkerTurnTimeout.resolve(
            global,
            project,
            BUILT_IN.worker_turn_timeout,
        )?,
        provider: Field::Provider.resolve(
            global,
            project,
            BUILT_IN.provider.map(str::to_string),
        )?,
        host: Field::Host.resolve(global, project, BUILT_IN.host.map(str::to_string))?,
        reply_style: Field::ReplyStyle.resolve(global, project, BUILT_IN.reply_style)?,
        theme: Field::Theme.resolve(global, project, BUILT_IN.theme)?,
        network_allowlist: Field::NetworkAllowlist.resolve(
            global,
            project,
            BUILT_IN
                .network_allowlist
                .iter()
                .map(|domain| (*domain).to_string())
                .collect(),
        )?,
        read_anywhere: Field::ReadAnywhere.resolve(global, project, BUILT_IN.read_anywhere)?,
        harness_global_folder: Field::HarnessGlobalFolder.resolve(
            global,
            project,
            BUILT_IN.harness_global_folder,
        )?,
        run_without_sandbox: Field::RunWithoutSandbox.resolve(
            global,
            project,
            BUILT_IN.run_without_sandbox,
        )?,
        bridge_updates: Field::BridgeUpdates.resolve(global, project, BTreeMap::new())?,
        lease_capacity: Field::LeaseCapacity.resolve(
            global,
            project,
            BUILT_IN
                .lease_capacity
                .iter()
                .map(|(name, capacity)| ((*name).to_string(), *capacity))
                .collect(),
        )?,
        pick_up: Field::PickUp.resolve(global, project, BUILT_IN.pick_up)?,
        ready_label: Field::ReadyLabel.resolve(
            global,
            project,
            BUILT_IN.ready_label.to_string(),
        )?,
        done_window: Field::DoneWindow.resolve(global, project, BUILT_IN.done_window)?,
    })
}

impl Field {
    /// The value at the nearest layer that sets this field
    ///
    /// A layer's file was checked against the schema before it got here, so a
    /// field only another layer may set is never in it.
    ///
    /// # Errors
    ///
    /// [`SettingsError::FileRefused`] naming the file and the field when the
    /// value does not fit the type the schema was written from.
    fn resolve<T>(
        self,
        global: &LayerDoc,
        project: &LayerDoc,
        built_in: T,
    ) -> Result<Layered<T>, SettingsError>
    where
        T: serde::de::DeserializeOwned,
    {
        for (layer, doc) in [(Layer::Project, project), (Layer::Global, global)] {
            let Some(value) = doc.fields.get(self.key()) else {
                continue;
            };
            let value = serde_json::from_value(value.clone()).map_err(|error| {
                SettingsError::FileRefused {
                    file: doc.file.clone(),
                    path: format!("/{}", self.key()),
                    reason: error.to_string(),
                }
            })?;
            return Ok(Layered { value, from: layer });
        }
        Ok(Layered {
            value: built_in,
            from: Layer::BuiltIn,
        })
    }
}
