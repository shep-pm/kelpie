//! The check that finds a bridge's newer version, installs it and probes it
//!
//! [`check`] is one pass of `docs/design/bridge-updates.md` (Flow): read the
//! registry's index, and when its stable version is newer than the one in use
//! and not marked bad, install it with [`ensure`] and probe it. What it finds is
//! written to `versions.json` ([`Versions`]). Nothing swaps here: a session
//! takes a `ready` version at its next idle point.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use semver::Version;

use crate::session::Harness;
use crate::settings::BridgeUpdates;
use crate::tree::ProcessTree;

use super::probe::{Capability, ProbeError, probe};
use super::registry::{self, URL};
use super::versions::{self, Rejected, Versions};
use super::{Pin, ensure};

/// What one [`check`] found
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CheckOutcome {
    /// The bridge is on kelpie's pin, so nothing was fetched
    Pinned,

    /// Nothing newer to try: the index's stable version is no newer than the one in
    /// use, or it is ready or marked bad already
    Current,

    /// The index gives no pin for this bridge: it is missing, quarantined, removed or
    /// has no binary for a platform kelpie ships on, so kelpie keeps what it has
    Unlisted {
        /// The version that stays in use
        in_use: Version,
    },

    /// A newer version is installed and passed the probe
    Ready {
        /// The version that is ready
        version: Version,
    },

    /// A newer version is installed and failed the probe
    HeldBack {
        /// The version held back
        version: Version,
        /// Why the probe failed it
        reason: String,
    },

    /// The check could not run, and nothing it wrote changed what is in use or ready
    Failed {
        /// What went wrong, for the log
        reason: String,
    },
}

/// Look for a newer version of `pin`'s bridge, install it and probe it
///
/// Under [`BridgeUpdates::KelpiesPin`] it fetches nothing. Otherwise it reads
/// [`registry::URL`], and a stable version newer than the one in use and not
/// marked bad is installed and probed against `requires`, then recorded as
/// `ready` or `held_back`. The probe starts the installed command with the args
/// and env the index's entry names for it. [`BridgeUpdates::RegistryAskFirst`]
/// checks the same way: asking is what a session does before it swaps. A failed
/// fetch or install changes nothing and is [`CheckOutcome::Failed`], for the
/// caller to log.
///
/// This blocks on the network and on the probe, so call it off the main thread.
#[must_use]
pub fn check(
    data_dir: &Path,
    tree: &Arc<ProcessTree>,
    pin: &Pin<'_>,
    policy: BridgeUpdates,
    requires: &[Capability],
) -> CheckOutcome {
    checked(URL, data_dir, pin, policy, &|harness| {
        probe(tree, harness, requires)
    })
}

/// [`check`], reading the index from `url` and probing with `probe`
pub(crate) fn checked(
    url: &str,
    data_dir: &Path,
    pin: &Pin<'_>,
    policy: BridgeUpdates,
    probe: &dyn Fn(&Harness) -> Result<(), ProbeError>,
) -> CheckOutcome {
    if policy == BridgeUpdates::KelpiesPin {
        return CheckOutcome::Pinned;
    }
    run(url, data_dir, pin, probe).unwrap_or_else(|reason| CheckOutcome::Failed { reason })
}

fn run(
    url: &str,
    data_dir: &Path,
    pin: &Pin<'_>,
    probe: &dyn Fn(&Harness) -> Result<(), ProbeError>,
) -> Result<CheckOutcome, String> {
    let mut versions = Versions::load(data_dir, pin).map_err(|error| error.to_string())?;
    let index = registry::fetch(url).map_err(|error| error.to_string())?;
    let Some(stable) = index.bridge(pin.registry_id) else {
        return Ok(CheckOutcome::Unlisted {
            in_use: versions.in_use,
        });
    };
    let candidate = versions::parse(stable.version).map_err(|error| error.to_string())?;
    // A kelpie release can ship a pin newer than the `in_use` an older run wrote,
    // and a version older than kelpie's own is no update
    let shipped = versions::parse(pin.version).map_err(|error| error.to_string())?;
    if candidate <= versions.in_use
        || candidate <= shipped
        || versions.ready.as_ref() == Some(&candidate)
        || versions.is_bad(&candidate)
    {
        return Ok(CheckOutcome::Current);
    }
    // The registry's id may differ from the folder kelpie keeps the bridge in
    let stable = Pin {
        id: pin.id,
        ..stable
    };
    let command = ensure(data_dir, &stable).map_err(|error| error.to_string())?;
    let outcome = match probe(&probe_harness(&stable, command)?) {
        Ok(()) => {
            versions.ready = Some(candidate.clone());
            versions.held_back.clear();
            CheckOutcome::Ready { version: candidate }
        }
        Err(error) => {
            let reason = error.to_string();
            versions.held_back = vec![Rejected {
                version: candidate.clone(),
                reason: reason.clone(),
            }];
            CheckOutcome::HeldBack {
                version: candidate,
                reason,
            }
        }
    };
    versions
        .save(data_dir, pin.id)
        .map_err(|error| error.to_string())?;
    Ok(outcome)
}

/// The [`Harness`] that runs `pin`'s installed bridge at `command`
///
/// The args and env are the index's own, so a bridge that is run with a
/// subcommand is probed the way it is run, and not as a bare command.
fn probe_harness(pin: &Pin<'_>, command: PathBuf) -> Result<Harness, String> {
    let download = pin.download().map_err(|error| error.to_string())?;
    Ok(Harness {
        program: command,
        args: download.args.iter().map(OsString::from).collect(),
        env: download
            .env
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect(),
        // The probe sends `initialize` alone, which asks the bridge for nothing
        new_session_meta: None,
        extensions: Vec::new(),
    })
}
