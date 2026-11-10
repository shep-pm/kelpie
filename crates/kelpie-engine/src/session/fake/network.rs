//! The sandbox a fake session runs in, which a `request_network` answer widens
//!
//! [`FakeNetwork`] stands for the network lists of a real sandbox
//! ([`crate::sandbox::Network`]): it holds hosts, takes the ones the maintainer
//! allows, and says which it lets through. A test hands one to
//! [`FakeAgent::sandbox_network`](super::FakeAgent::sandbox_network) and reads
//! [`FakeNetwork::allowed`] to see what an answer did.

use std::sync::{Arc, Mutex};

use crate::tools::Reach;

use super::lock;

/// The hosts a fake session's sandbox lets through
///
/// Cheap to clone: every clone holds the same lists.
#[derive(Debug, Clone, Default)]
pub struct FakeNetwork {
    hosts: Arc<Mutex<Vec<String>>>,
}

impl FakeNetwork {
    /// One that already lets `allowed` through
    #[must_use]
    pub fn allowing(allowed: &[&str]) -> Self {
        Self {
            hosts: Arc::new(Mutex::new(
                allowed.iter().map(|host| (*host).to_owned()).collect(),
            )),
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
        lock(&self.hosts).push(host.to_owned());
        true
    }
}
