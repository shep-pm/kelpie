//! The bridge row on the settings page
//!
//! Kelpie installs each bridge from the ACP registry and looks for a newer version
//! every 24 hours (`crate::state::AppState::start_bridge_checks`). The row reads
//! what those checks wrote: the version sessions start on, where it came from, and
//! every version held back or marked bad with its reason. [`bridge_approve`] is
//! the *Registry, ask first* yes, over the engine's [`Versions::approve`]
//! (`docs/design/bridge-updates.md`, The setting).
//!
//! Nothing here installs, probes or swaps anything, so the row starts no process
//! (ADR 0001). The setting itself is the `bridge-updates` field of the settings
//! form (`crate::settings`), which is why the row is not a form field: one field
//! holds one value per bridge.

use kelpie_engine::bridge::{Pin, Rejected, Versions};
use kelpie_engine::harness::claude;
use semver::Version;
use serde::Serialize;
use tauri::State;

use crate::state::AppState;

/// Where the version a bridge runs on came from
// wire format: the bindings hold these two strings
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum BridgeSource {
    /// A newer version kelpie took from the ACP registry
    Registry,

    /// The version written in kelpie's code, which changes with kelpie's releases
    KelpiesPin,
}

/// One version of a bridge kelpie did not use, and why
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct RejectedView {
    /// The version, as a person reads it
    pub version: String,

    /// What went wrong with it, as the person reads it
    pub reason: String,
}

/// What the settings page says about one bridge
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct BridgeStatus {
    /// The version sessions start on
    pub in_use: String,

    /// Where that version came from
    pub source: BridgeSource,

    /// A newer version that is installed and passed the probe, `None` when none waits
    pub ready: Option<String>,

    /// The ready version the maintainer said yes to, `None` when none waits approved
    pub approved: Option<String>,

    /// Versions that failed the probe, each with its reason
    pub held_back: Vec<RejectedView>,

    /// Versions that failed a session, each with its reason
    pub bad: Vec<RejectedView>,
}

/// Every bridge kelpie installs, each pinned in kelpie's code
///
/// A bridge's `id` is the one its versions and installs are kept under, which is
/// also the key its update setting is keyed by.
fn pins() -> [&'static Pin<'static>; 1] {
    [claude::pin()]
}

/// The pin of the bridge `id` names
///
/// # Errors
///
/// `id` names no bridge kelpie installs.
fn pin_of(id: &str) -> Result<&'static Pin<'static>, String> {
    pins()
        .into_iter()
        .find(|pin| pin.id == id)
        .ok_or_else(|| format!("kelpie has no bridge named {id}"))
}

/// What `id`'s bridge runs on, and what waits behind it
///
/// With no `versions.json` the version in use is kelpie's pin, which is what the
/// row says came from kelpie's pin. A version past it is one the registry gave.
/// Nothing here installs or starts anything: the row reads what the bridge checks
/// already wrote (`docs/design/bridge-updates.md`, The setting).
///
/// # Errors
///
/// `id` names no bridge kelpie installs, or its versions cannot be read.
#[tauri::command(async)]
#[specta::specta]
pub fn bridge_status(id: String, state: State<'_, AppState>) -> Result<BridgeStatus, String> {
    let pin = pin_of(&id)?;
    let versions = Versions::load(state.data_dir(), pin).map_err(|error| error.to_string())?;
    // A pin that will not parse is kelpie's own bug, and says so rather than
    // guessing where the version in use came from
    let shipped = Version::parse(pin.version).map_err(|error| error.to_string())?;
    let approved = versions
        .approved
        .clone()
        .filter(|approved| versions.ready.as_ref() == Some(approved))
        .map(|version| version.to_string());
    Ok(BridgeStatus {
        in_use: versions.in_use.to_string(),
        source: if versions.in_use == shipped {
            BridgeSource::KelpiesPin
        } else {
            BridgeSource::Registry
        },
        ready: versions.ready.map(|version| version.to_string()),
        approved,
        held_back: rejected(versions.held_back),
        bad: rejected(versions.bad),
    })
}

/// Versions the bridge did not use, as the row shows them
fn rejected(versions: Vec<Rejected>) -> Vec<RejectedView> {
    versions
        .into_iter()
        .map(|rejected| RejectedView {
            version: rejected.version.to_string(),
            reason: rejected.reason,
        })
        .collect()
}

/// Say the version ready now may be used, for *Registry, ask first*
///
/// Only the version ready when this is called is approved: a newer one a later
/// check finds asks again. The swap itself happens at each session's next idle
/// point, which is what the row says after the yes.
///
/// # Errors
///
/// `id` names no bridge kelpie installs, or its versions cannot be read or written.
#[tauri::command(async)]
#[specta::specta]
pub fn bridge_approve(id: String, state: State<'_, AppState>) -> Result<Option<String>, String> {
    let pin = pin_of(&id)?;
    let approved = Versions::update(state.data_dir(), pin, |versions| versions.approve())
        .map_err(|error| error.to_string())?;
    Ok(approved.map(|version| version.to_string()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use kelpie_engine::harness::claude;
    use kelpie_engine::login::Store;
    use kelpie_engine::run::RunId;
    use kelpie_engine::settings::ConfigDir;

    use super::{BridgeSource, bridge_approve, bridge_status};
    use crate::state::AppState;
    use tauri::Manager as _;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    /// The app on a data folder of its own, with no project and no reaper
    fn app() -> (PathBuf, tauri::App<tauri::test::MockRuntime>) {
        let dir = std::env::temp_dir().join(format!("kelpie-bridges-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        let state = AppState::open(
            dir.clone(),
            ConfigDir::at(dir.join("config")),
            Store::memory(),
            Box::new(|_| Err("never built".to_owned())),
            Box::new(|| Ok(None)),
            None,
        )
        .unwrap();
        (
            dir,
            mock_builder()
                .manage(state)
                .build(mock_context(noop_assets()))
                .unwrap(),
        )
    }

    /// What the bridge checks wrote for one bridge, as `versions.json` holds it
    fn write_versions(dir: &std::path::Path, text: &str) {
        let bridge = dir.join("bridges").join(claude::pin().id);
        fs::create_dir_all(&bridge).unwrap();
        fs::write(bridge.join("versions.json"), text).unwrap();
    }

    #[test]
    fn a_bridge_row_says_the_version_in_use_and_where_it_came_from() {
        let (dir, app) = app();
        let state = app.state::<AppState>();

        // Nothing has been checked yet, so the row says kelpie's own pin
        let fresh = bridge_status(claude::pin().id.into(), state.clone()).unwrap();
        assert_eq!(fresh.in_use, claude::pin().version);
        assert_eq!(fresh.source, BridgeSource::KelpiesPin);
        assert_eq!(fresh.ready, None);
        assert_eq!(fresh.approved, None);
        assert!(fresh.held_back.is_empty() && fresh.bad.is_empty());

        write_versions(
            &dir,
            r#"{"in_use":"0.102.0","previous":"0.101.1","ready":"0.103.0","approved":"0.102.0"}"#,
        );
        let swapped = bridge_status(claude::pin().id.into(), state.clone()).unwrap();
        assert_eq!(swapped.in_use, "0.102.0");
        assert_eq!(swapped.source, BridgeSource::Registry);
        assert_eq!(swapped.ready.as_deref(), Some("0.103.0"));
        assert_eq!(swapped.approved, None);

        write_versions(
            &dir,
            r#"{"in_use":"0.102.0","previous":"0.101.1","ready":"0.103.0","approved":"0.103.0"}"#,
        );
        let asked = bridge_status(claude::pin().id.into(), state.clone()).unwrap();
        assert_eq!(asked.approved.as_deref(), Some("0.103.0"));

        let error = bridge_status("codex-acp".into(), state.clone()).unwrap_err();
        assert!(error.contains("codex-acp"), "{error}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_held_back_or_bad_version_keeps_its_reason() {
        let (dir, app) = app();
        let state = app.state::<AppState>();
        write_versions(
            &dir,
            r#"{
                "in_use": "0.101.1",
                "held_back": [{"version": "0.102.0", "reason": "it never answered initialize"}],
                "bad": [{"version": "0.101.2", "reason": "the session could not resume"}]
            }"#,
        );

        let seen = bridge_status(claude::pin().id.into(), state.clone()).unwrap();

        assert_eq!(seen.held_back.len(), 1);
        assert_eq!(seen.held_back[0].version, "0.102.0");
        assert_eq!(seen.held_back[0].reason, "it never answered initialize");
        assert_eq!(seen.bad[0].version, "0.101.2");
        assert_eq!(seen.bad[0].reason, "the session could not resume");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_update_says_yes_to_the_version_ready_now() {
        let (dir, app) = app();
        let state = app.state::<AppState>();
        write_versions(&dir, r#"{"in_use":"0.101.1","ready":"0.102.0"}"#);

        let approved = bridge_approve(claude::pin().id.into(), state.clone()).unwrap();

        assert_eq!(approved.as_deref(), Some("0.102.0"));
        let held = fs::read_to_string(
            dir.join("bridges")
                .join(claude::pin().id)
                .join("versions.json"),
        )
        .unwrap();
        assert!(held.contains(r#""approved": "0.102.0""#), "{held}");
        // The version in use only moves at a session's next idle point, and the
        // row reads the yes back out of the file rather than holding it itself
        let seen = bridge_status(claude::pin().id.into(), state.clone()).unwrap();
        assert_eq!(seen.in_use, claude::pin().version);
        assert_eq!(seen.ready.as_deref(), Some("0.102.0"));
        assert_eq!(seen.approved.as_deref(), Some("0.102.0"));

        // Nothing ready: the yes answers `None` rather than approving a guess
        write_versions(&dir, r#"{"in_use":"0.101.1"}"#);
        assert_eq!(
            bridge_approve(claude::pin().id.into(), state.clone()).unwrap(),
            None
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
