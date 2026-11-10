//! A scripted bridge, for a session that moves between its versions
//!
//! [`FakeBridge`] answers the way [`crate::bridge::Versions`] does, so a test of
//! a session on a [`Bridge`] needs no registry, no download and no probe
//! (`docs/design/bridge-updates.md`, Swapping).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use semver::Version;

use super::{FakeAgent, lock};
use crate::session::{Bridge, Started};

/// The name a [`FakeBridge`] divider gives it
pub const BRIDGE: &str = "fake-agent-acp";

/// A bridge with a scripted agent on each of its versions
///
/// A ready version is taken into use at the next look and the version before it
/// is kept, and a version that failed a session is marked bad and the one before
/// it is used again.
#[derive(Debug, Clone)]
pub struct FakeBridge {
    shared: Arc<BridgeShared>,
}

#[derive(Debug)]
struct BridgeShared {
    /// The version a session starts on
    in_use: Mutex<Version>,
    /// The version before it, which one that failed a session goes back to
    previous: Mutex<Option<Version>>,
    /// The newer version waiting to be taken into use
    ready: Mutex<Option<Version>>,
    /// Each version kelpie said failed a session, with why, oldest first
    bad: Mutex<Vec<(Version, String)>>,
    /// The agent to start on each version
    agents: Mutex<HashMap<Version, FakeAgent>>,
    /// Every version a session started on, oldest first
    started: Mutex<Vec<Version>>,
}

impl FakeBridge {
    /// A bridge on `version`, with no agent on it yet
    #[must_use]
    pub fn new(version: &str) -> Self {
        Self {
            shared: Arc::new(BridgeShared {
                in_use: Mutex::new(bridge_version(version)),
                previous: Mutex::new(None),
                ready: Mutex::new(None),
                bad: Mutex::new(Vec::new()),
                agents: Mutex::new(HashMap::new()),
                started: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Start `agent` on `version`
    #[must_use]
    pub fn agent(self, version: &str, agent: FakeAgent) -> Self {
        lock(&self.shared.agents).insert(bridge_version(version), agent);
        self
    }

    /// Say `version` is installed and passed the probe, and no session has taken it yet
    #[must_use]
    pub fn ready(self, version: &str) -> Self {
        *lock(&self.shared.ready) = Some(bridge_version(version));
        self
    }

    /// The [`Bridge`] a session runs on
    #[must_use]
    pub fn bridge(&self) -> Bridge {
        let (current, start, failed) = (
            Arc::clone(&self.shared),
            Arc::clone(&self.shared),
            Arc::clone(&self.shared),
        );
        Bridge {
            id: BRIDGE.to_owned(),
            current: Box::new(move || {
                let ready = lock(&current.ready).clone();
                match ready {
                    Some(ready) => {
                        *lock(&current.ready) = None;
                        let was = std::mem::replace(&mut *lock(&current.in_use), ready.clone());
                        *lock(&current.previous) = Some(was);
                        ready
                    }
                    None => lock(&current.in_use).clone(),
                }
            }),
            start: Box::new(move |version| {
                lock(&start.started).push(version.clone());
                let agent = lock(&start.agents)
                    .get(version)
                    .cloned()
                    .ok_or_else(|| format!("no fake agent on {version}"))?;
                Ok(Started {
                    harness: agent.harness(),
                    launch: Box::new(agent),
                })
            }),
            failed: Box::new(move |version, reason| {
                lock(&failed.bad).push((version.clone(), reason.to_owned()));
                let ready = lock(&failed.ready).clone();
                if ready.as_ref() == Some(version) {
                    *lock(&failed.ready) = None;
                }
                let in_use = lock(&failed.in_use).clone();
                if in_use != *version {
                    return None;
                }
                let back = lock(&failed.previous).take()?;
                *lock(&failed.in_use) = back.clone();
                Some(back)
            }),
        }
    }

    /// Every version a session started on, oldest first
    #[must_use]
    pub fn started(&self) -> Vec<Version> {
        lock(&self.shared.started).clone()
    }

    /// Each version kelpie said failed a session, with why, oldest first
    #[must_use]
    pub fn bad(&self) -> Vec<(Version, String)> {
        lock(&self.shared.bad).clone()
    }

    /// The version a session starts on now
    #[must_use]
    pub fn in_use(&self) -> Version {
        lock(&self.shared.in_use).clone()
    }

    /// The version before the one in use
    #[must_use]
    pub fn previous(&self) -> Option<Version> {
        lock(&self.shared.previous).clone()
    }
}

/// A test's version, which is always one
fn bridge_version(text: &str) -> Version {
    Version::parse(text).expect("a test bridge version is a semantic version")
}
