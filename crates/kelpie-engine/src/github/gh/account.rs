//! Whose login `gh` holds, and what is left of its rate limits

use serde::Deserialize;

use super::{Gh, unreadable};
use crate::github::Timestamp;
use crate::ticket::ForgeError;

/// One rate limit as GitHub counts it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateBucket {
    /// The calls it has counted since it last reset
    pub used: u64,
    /// The calls it has left before it is used up
    pub remaining: u64,
    /// When it resets and starts counting again
    pub reset: Timestamp,
}

/// The limits the maintainer's login spends from
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimits {
    /// The REST API's
    pub core: RateBucket,
    /// The GraphQL API's
    pub graphql: RateBucket,
    /// The search API's, which `gh search` spends from
    pub search: RateBucket,
}

impl Gh {
    /// The login `gh` holds, from `gh api user`
    ///
    /// # Errors
    ///
    /// As [`Gh::call`], or [`ForgeError::Unreadable`] when what it printed is
    /// not one word.
    pub fn viewer(&self) -> Result<String, ForgeError> {
        let stdout = self.call(&["api", "user", "--jq", ".login"])?;
        let login = String::from_utf8_lossy(&stdout).trim().to_owned();
        let plain = !login.is_empty() && !login.contains(char::is_whitespace);
        plain.then_some(login).ok_or_else(|| unreadable(&stdout))
    }

    /// The login's `core`, `graphql` and `search` limits as GitHub counts them
    /// now, from `gh api rate_limit`, which counts against none of them
    ///
    /// # Errors
    ///
    /// As [`Gh::call`], or [`ForgeError::Unreadable`] when the answer lacks a limit.
    pub fn rate_limit(&self) -> Result<RateLimits, ForgeError> {
        let stdout = self.call(&["api", "rate_limit"])?;
        parse_rate_limit(&stdout)
    }
}

fn parse_rate_limit(stdout: &[u8]) -> Result<RateLimits, ForgeError> {
    #[derive(Deserialize)]
    struct Limits {
        resources: Resources,
    }
    #[derive(Deserialize)]
    struct Resources {
        core: Limit,
        graphql: Limit,
        search: Limit,
    }
    #[derive(Deserialize)]
    struct Limit {
        used: u64,
        remaining: u64,
        reset: u64,
    }
    let bucket = |limit: Limit| RateBucket {
        used: limit.used,
        remaining: limit.remaining,
        reset: Timestamp::from_secs(limit.reset),
    };
    let limits: Limits = serde_json::from_slice(stdout).map_err(|_| unreadable(stdout))?;
    Ok(RateLimits {
        core: bucket(limits.resources.core),
        graphql: bucket(limits.resources.graphql),
        search: bucket(limits.resources.search),
    })
}
