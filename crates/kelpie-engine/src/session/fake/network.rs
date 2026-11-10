//! The sandbox a fake session runs in, which a `request_network` answer widens
//!
//! [`FakeNetwork`] stands for the network lists of a real sandbox
//! ([`crate::sandbox::Network`]): it holds hosts, takes the ones the maintainer
//! allows, and says which it lets through. A test hands one to
//! [`FakeAgent::sandbox_network`](super::FakeAgent::sandbox_network) and reads
//! [`FakeNetwork::allowed`] to see what an answer did. One built with
//! [`FakeNetwork::gone`] takes nothing, as a dropped sandbox takes nothing.

use std::sync::{Arc, Mutex};

use crate::tools::Reach;

use super::lock;

/// The hosts a fake session's sandbox lets through
///
/// Cheap to clone: every clone holds the same lists.
#[derive(Debug, Clone)]
pub struct FakeNetwork {
    hosts: Arc<Mutex<Vec<String>>>,
    /// Whether it takes a host an answer allows. `false` stands for a sandbox that is gone.
    takes: bool,
}

impl Default for FakeNetwork {
    fn default() -> Self {
        Self {
            hosts: Arc::new(Mutex::new(Vec::new())),
            takes: true,
        }
    }
}

impl FakeNetwork {
    /// One that already lets `allowed` through
    #[must_use]
    pub fn allowing(allowed: &[&str]) -> Self {
        Self {
            hosts: Arc::new(Mutex::new(
                allowed.iter().map(|host| (*host).to_owned()).collect(),
            )),
            ..Self::default()
        }
    }

    /// One that takes nothing, as a sandbox that has dropped takes nothing
    #[must_use]
    pub fn gone() -> Self {
        Self {
            takes: false,
            ..Self::default()
        }
    }

    /// The hosts it lets through now, oldest first
    #[must_use]
    pub fn allowed(&self) -> Vec<String> {
        lock(&self.hosts).clone()
    }
}

impl Reach for FakeNetwork {
    fn allows(&self, host: &str) -> bool {
        self.allowed().iter().any(|allowed| allowed == host)
    }

    fn allow(&self, host: &str) -> bool {
        if !self.takes {
            return false;
        }
        lock(&self.hosts).push(host.to_owned());
        true
    }
}
