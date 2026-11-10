//! The ACP registry's own index, read for the bridges kelpie installs
//!
//! [`fetch`] reads the one file the registry publishes for every agent,
//! `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`, and
//! [`Index::bridge`] turns one agent's stable `version` and `distribution.binary`
//! into a [`Pin`] that [`super::ensure`] installs. Preview channels are never
//! read, and an entry kelpie cannot use is left out, so the bridge it names has
//! no pin and kelpie keeps what it has (`docs/design/bridge-updates.md`, Flow).

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use super::{Download, Pin, Platform};

/// Where the registry publishes its index
pub const URL: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// How long one read of the index may take end to end
const TIMEOUT: Duration = Duration::from_secs(30);

/// The most bytes kelpie reads from the index
///
/// 16 MiB, far above the few hundred kilobytes the registry's index is, so a
/// server that answers wrong cannot fill memory.
const LIMIT: u64 = 16 * 1024 * 1024;

/// Why the registry's index could not be read
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug)]
#[non_exhaustive]
pub enum RegistryError {
    /// The index could not be fetched, or the server answered an error status
    Fetch {
        /// The url kelpie fetched
        url: String,
        /// What the client said
        source: io::Error,
    },

    /// The index was fetched but is not the JSON object with an `agents` list kelpie reads
    Malformed {
        /// The url it came from
        url: String,
        /// What the parser said
        message: String,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fetch { url, source } => {
                write!(f, "cannot fetch the registry at {url}: {source}")
            }
            Self::Malformed { url, message } => {
                write!(
                    f,
                    "the registry at {url} is not an index kelpie reads: {message}"
                )
            }
        }
    }
}

impl core::error::Error for RegistryError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Fetch { source, .. } => Some(source),
            Self::Malformed { .. } => None,
        }
    }
}

/// The registry's index: each agent's stable version and the archives it ships as
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    agents: Vec<Agent>,
}

/// One agent of the index that kelpie can install: it ships a binary archive
#[derive(Debug, Clone, PartialEq, Eq)]
struct Agent {
    id: String,
    version: String,
    platforms: Vec<(Platform, Download)>,
}

impl Index {
    /// The pin for the agent `registry_id`, at the version the index calls stable
    ///
    /// `None` when the index does not list the agent, or lists it with no binary
    /// for the platform this process runs on. That covers an agent the registry
    /// quarantined or removed, and kelpie then keeps the version it has.
    #[must_use]
    pub fn bridge(&self, registry_id: &str) -> Option<Pin<'_>> {
        let agent = self.agents.iter().find(|agent| agent.id == registry_id)?;
        let current = Platform::current()?;
        agent
            .platforms
            .iter()
            .any(|(platform, _)| *platform == current)
            .then_some(Pin {
                id: &agent.id,
                registry_id: &agent.id,
                version: &agent.version,
                platforms: &agent.platforms,
            })
    }

    /// Read the index from its JSON text
    fn parse(text: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Document {
            agents: Vec<Value>,
        }
        let document: Document = serde_json::from_str(text).map_err(|error| error.to_string())?;
        // An entry that is not shaped as kelpie reads one is no agent of its own,
        // and says nothing about the others
        let agents = document
            .agents
            .into_iter()
            .filter_map(|agent| serde_json::from_value::<Wire>(agent).ok())
            .map(Wire::agent)
            .collect();
        Ok(Self { agents })
    }
}

/// One agent as the registry writes it
#[derive(Deserialize)]
struct Wire {
    id: String,
    version: String,
    #[serde(default)]
    distribution: Distribution,
}

#[derive(Default, Deserialize)]
struct Distribution {
    /// Each platform's archive, keyed by the registry's platform name
    #[serde(default)]
    binary: BTreeMap<String, Value>,
}

/// One platform's archive as the registry writes it
#[derive(Deserialize)]
struct Binary {
    archive: String,
    cmd: String,
    sha256: String,
}

impl Wire {
    fn agent(self) -> Agent {
        // An archive with no sha256 is never installed, since nothing could check it
        let platforms = self
            .distribution
            .binary
            .into_iter()
            .filter_map(|(name, binary)| {
                let platform = platform(&name)?;
                let binary: Binary = serde_json::from_value(binary).ok()?;
                let cmd = binary.cmd.strip_prefix("./").unwrap_or(&binary.cmd);
                Some((
                    platform,
                    Download {
                        url: binary.archive,
                        sha256: binary.sha256,
                        cmd: cmd.to_owned(),
                    },
                ))
            })
            .collect();
        Agent {
            id: self.id,
            version: self.version,
            platforms,
        }
    }
}

/// The platform the registry's name stands for, `None` for one kelpie does not ship on
fn platform(name: &str) -> Option<Platform> {
    match name {
        "linux-x86_64" => Some(Platform::LinuxX64),
        "linux-aarch64" => Some(Platform::LinuxArm64),
        "darwin-x86_64" => Some(Platform::MacOsX64),
        "darwin-aarch64" => Some(Platform::MacOsArm64),
        "windows-x86_64" => Some(Platform::WindowsX64),
        _ => None,
    }
}

/// Read the registry's index from `url`
///
/// Kelpie's own client does the fetch, so no downloader enters the process tree
/// (ADR 0001). Use [`URL`] for the registry itself.
///
/// # Errors
///
/// [`RegistryError::Fetch`] when the server cannot be reached or answers an
/// error status, and [`RegistryError::Malformed`] when what it sends is not an
/// index.
pub fn fetch(url: &str) -> Result<Index, RegistryError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let unreachable = |error: ureq::Error| RegistryError::Fetch {
        url: url.to_owned(),
        source: io::Error::other(error),
    };
    let text = agent
        .get(url)
        .call()
        .map_err(unreachable)?
        .into_body()
        .into_with_config()
        .limit(LIMIT)
        .read_to_string()
        .map_err(unreachable)?;
    Index::parse(&text).map_err(|message| RegistryError::Malformed {
        url: url.to_owned(),
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An index whose one agent ships a binary for `platform` only
    fn index_with_only(platform: &str) -> Index {
        let text = format!(
            r#"{{"agents": [{{"id": "claude-acp", "version": "0.102.0", "distribution": {{"binary": {{
                "{platform}": {{"archive": "https://example.test/a.zip", "cmd": "./a", "sha256": "00"}}
            }}}}}}]}}"#
        );
        Index::parse(&text).unwrap()
    }

    /// The registry's name for the platform these tests run on
    fn this_platform() -> &'static str {
        match Platform::current().expect("a platform kelpie ships on") {
            Platform::LinuxX64 => "linux-x86_64",
            Platform::LinuxArm64 => "linux-aarch64",
            Platform::MacOsX64 => "darwin-x86_64",
            Platform::MacOsArm64 => "darwin-aarch64",
            Platform::WindowsX64 => "windows-x86_64",
        }
    }

    #[test]
    fn a_bridge_published_for_other_platforms_only_has_no_pin() {
        let other = if this_platform() == "windows-x86_64" {
            "linux-x86_64"
        } else {
            "windows-x86_64"
        };

        assert!(index_with_only(other).bridge("claude-acp").is_none());
    }

    #[test]
    fn a_bridge_published_for_this_platform_has_a_pin() {
        let index = index_with_only(this_platform());

        assert_eq!(index.bridge("claude-acp").unwrap().version, "0.102.0");
    }
}
