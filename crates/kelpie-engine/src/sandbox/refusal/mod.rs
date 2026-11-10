//! Why bwrap refused to start a sandbox, read from its own words

use std::fmt;
use std::process::Output;

use super::SandboxError;

/// One of bwrap's two refusals to start a sandbox
// kelpie-engine is a library: an out-of-tree match would break on a new refusal
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// bwrap made its user namespace and the kernel gave it no capabilities
    /// in it: `bwrap: loopback: Failed RTM_NEWADDR`
    NoCapabilities,

    /// The kernel lets bwrap make no user namespace at all:
    /// `bwrap: No permissions to create new namespace`
    NoNamespaces,
}

impl Refusal {
    /// The refusal bwrap's `stderr` names on its first line, or `None`
    #[must_use]
    pub fn of(stderr: &str) -> Option<Self> {
        let first = stderr.lines().next()?;
        if first.starts_with("bwrap: loopback: Failed RTM_NEWADDR") {
            Some(Self::NoCapabilities)
        } else if first.starts_with("bwrap: No permissions to create new namespace") {
            Some(Self::NoNamespaces)
        } else {
            None
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoCapabilities => {
                "bwrap's user namespace has no capabilities, which on Ubuntu 24.04 and later is the host's AppArmor restriction on user namespaces"
            }
            Self::NoNamespaces => "the kernel does not allow user namespaces, so bwrap can make none",
        })
    }
}

impl core::error::Error for Refusal {}

/// Read what the probe command printed inside the sandbox
///
/// # Errors
///
/// [`SandboxError::Refused`] when bwrap named one of its [`Refusal`]s, and
/// [`SandboxError::Probe`], with stderr, for any other failure.
pub(super) fn judge(output: &Output) -> Result<(), SandboxError> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(match Refusal::of(&stderr) {
        Some(refusal) => SandboxError::Refused(refusal),
        None => SandboxError::Probe(stderr.into_owned()),
    })
}

#[cfg(test)]
mod tests;
