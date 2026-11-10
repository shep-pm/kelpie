//! The GitHub calls an App makes, and the signer its tokens are asked with
//!
//! [`GithubApi`] is the port: [`super::curl::CurlGithub`] runs `curl`, and a test
//! a stand-in GitHub. The answers' JSON is read here, from the shapes GitHub's
//! REST documentation gives for each call. Only the two calls minting a token
//! are here; the writes an App makes as itself come with the forge slice.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde_json::Value;

use super::slug::ForgeSlug;
use super::tokens::Jwt;
use super::{InstallationToken, Timestamp};

/// An App's private key in PEM, as the conversion hands it back
///
/// `Debug` does not show it.
#[derive(Clone, PartialEq, Eq)]
pub struct Pem(String);

impl Pem {
    /// The key `text`
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The key's text, for the one file kelpie keeps it in
    #[inline]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Pem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Pem(..)")
    }
}

/// An App's installation on a repo, as GitHub describes it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Installation {
    /// The installation's id
    pub id: u64,
    /// Whether the installation holds `workflows` at `write`: the App was
    /// given it and the installation accepted it
    pub workflows: bool,
}

/// An installation token and when GitHub lets it lapse
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedToken {
    /// The token
    pub token: super::InstallationToken,
    /// When it stops working
    pub expires_at: Timestamp,
    /// What GitHub says it holds, such as `contents` to `write`
    pub permissions: BTreeMap<String, String>,
}

impl IssuedToken {
    /// Whether the token still has `remint_before` seconds or more to run at `now`
    pub(crate) fn holds_for(&self, now: Timestamp, remint_before: u64) -> bool {
        now.secs().saturating_add(remint_before) < self.expires_at.secs()
    }
}

/// What an answer's rate-limit headers said of when to ask again
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cooldown {
    /// Seconds to wait from the answer, from `Retry-After`
    pub retry_after: Option<u64>,
    /// When the limit resets, from `X-RateLimit-Reset`
    pub reset: Option<Timestamp>,
}

impl Cooldown {
    /// What the two headers' values say: each a whole number, anything else
    /// (an HTTP date in `Retry-After`, an empty value) read as missing
    pub fn read(retry_after: &str, reset: &str) -> Self {
        let number = |text: &str| text.trim().parse::<u64>().ok();
        Self {
            retry_after: number(retry_after),
            reset: number(reset).map(Timestamp::from_secs),
        }
    }
}

/// Why a call to GitHub came to nothing
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApiError {
    /// GitHub could not be reached, with why
    Unreachable(String),
    /// GitHub answered with this HTTP status, not the one the call wants
    Refused(u16),
    /// GitHub answered 429, or 403 saying a rate limit was passed, with when
    /// its headers say to ask again
    RateLimited(Cooldown),
    /// GitHub answered 301: the repo was renamed or moved, and kelpie never
    /// follows a redirect with a JWT
    Moved,
    /// GitHub's answer lacked the field named
    Unreadable(&'static str),
}

impl ApiError {
    /// Whether asking again later may succeed with nothing changed
    #[must_use]
    pub fn passes(&self) -> bool {
        match self {
            Self::Unreachable(_) | Self::RateLimited(_) => true,
            Self::Refused(status) => *status >= 500,
            Self::Moved | Self::Unreadable(_) => false,
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(why) => write!(f, "cannot reach GitHub: {why}"),
            Self::Refused(status) => write!(f, "GitHub answered HTTP {status}"),
            Self::RateLimited(_) => f.write_str("GitHub's rate limit for the App is spent for now"),
            Self::Moved => f.write_str(
                "GitHub says the repo moved: put its new `owner/name` in the project's remote",
            ),
            Self::Unreadable(field) => write!(f, "GitHub's answer has no readable `{field}`"),
        }
    }
}

impl core::error::Error for ApiError {}

/// The GitHub calls an App makes to mint a token
pub trait GithubApi: Send + Sync {
    /// The App's installation on `repo`, `GET /repos/{owner}/{repo}/installation`,
    /// or `None` when GitHub says the App is not installed there
    ///
    /// # Errors
    ///
    /// [`ApiError`] when GitHub cannot be reached or refuses the App's `jwt`.
    fn installation(&self, jwt: &Jwt, repo: &ForgeSlug) -> Result<Option<Installation>, ApiError>;

    /// A new token for `installation` that reaches `repo` alone and holds
    /// the permissions `grant` names,
    /// `POST /app/installations/{id}/access_tokens` with
    /// `{"repositories": [<name>], "permissions": {..}}`
    ///
    /// # Errors
    ///
    /// [`ApiError`] when GitHub cannot be reached or refuses the App's `jwt`, or
    /// refuses a permission the App was never given with 422.
    fn access_token(
        &self,
        jwt: &Jwt,
        installation: u64,
        repo: &ForgeSlug,
        grant: Grant,
    ) -> Result<IssuedToken, ApiError>;
}

/// Signs with an App's private key: RS256, an RSA signature over SHA-256
pub trait Signer: Send + Sync {
    /// The signature of `input` by the key in the file `key`
    ///
    /// # Errors
    ///
    /// Why the key could not sign.
    fn sign(&self, key: &Path, input: &[u8]) -> Result<Vec<u8>, String>;
}

/// The error for an answer of `status` that is not the call's success,
/// telling a rate limit from a refusal by `body`, and carrying what the
/// answer's headers said of when to ask again as `cooldown`
pub fn refusal(status: u16, body: &str, cooldown: Cooldown) -> ApiError {
    match status {
        301 => ApiError::Moved,
        429 => ApiError::RateLimited(cooldown),
        403 if body.to_ascii_lowercase().contains("rate limit") => ApiError::RateLimited(cooldown),
        status => ApiError::Refused(status),
    }
}

/// What an installation token may do, which is less than the App may
///
/// A token asks for permissions the App holds and gets those alone, so a
/// token is never more than the call that uses it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// Kelpie's own posts: issues and pull requests, written
    Kelpie,
    /// A worker's: contents, written, which lets it push its branch, and
    /// workflows, written, where the installation holds it, which lets it
    /// change a workflow file. Nothing else: no comment, review, approval or
    /// merge.
    Worker {
        /// Whether the token asks for `workflows` too
        workflows: bool,
    },
}

impl Grant {
    /// What the token is kept for until it nears its end, or none when it is
    /// minted for one call. A worker's is never kept: it can push, so kelpie's
    /// own posts must not be handed it.
    pub(crate) fn kept_for(self) -> Option<Purpose> {
        match self {
            Self::Kelpie => Some(Purpose::Posts),
            Self::Worker { .. } => None,
        }
    }
}

/// What a kept token is for
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Purpose {
    /// Kelpie's posts
    Posts,
}

/// The body of an access token's request, which limits the token to `repo`
/// and to what `grant` allows
pub fn token_request(repo: &ForgeSlug, grant: Grant) -> String {
    let permissions = match grant {
        Grant::Kelpie => serde_json::json!({ "issues": "write", "pull_requests": "write" }),
        Grant::Worker { workflows: false } => serde_json::json!({ "contents": "write" }),
        Grant::Worker { workflows: true } => {
            serde_json::json!({ "contents": "write", "workflows": "write" })
        }
    };
    serde_json::json!({ "permissions": permissions, "repositories": [repo.name()] }).to_string()
}

/// The installation in `GET /repos/{owner}/{repo}/installation`'s answer
///
/// # Errors
///
/// [`ApiError::Unreadable`] when it has no id.
pub fn parse_installation(body: &str) -> Result<Installation, ApiError> {
    let v: Value = serde_json::from_str(body).map_err(|_| ApiError::Unreadable("id"))?;
    Ok(Installation {
        id: v["id"].as_u64().ok_or(ApiError::Unreadable("id"))?,
        workflows: v["permissions"]["workflows"].as_str() == Some("write"),
    })
}

/// The token in an access token's answer, and when it lapses
///
/// # Errors
///
/// [`ApiError::Unreadable`] naming the field missing or unreadable.
pub fn parse_token(body: &str) -> Result<IssuedToken, ApiError> {
    let v: Value = serde_json::from_str(body).map_err(|_| ApiError::Unreadable("token"))?;
    let token =
        (v["token"].as_str().filter(|t| !t.is_empty())).ok_or(ApiError::Unreadable("token"))?;
    let at = (v["expires_at"].as_str())
        .and_then(epoch_seconds)
        .ok_or(ApiError::Unreadable("expires_at"))?;
    let permissions = (v["permissions"].as_object().into_iter().flatten())
        .filter_map(|(name, level)| Some((name.clone(), level.as_str()?.to_owned())))
        .collect();
    Ok(IssuedToken {
        token: InstallationToken::new(token.to_owned()),
        expires_at: at,
        permissions,
    })
}

// GitHub's times are UTC whole seconds, `YYYY-MM-DDTHH:MM:SSZ`, which is the
// only shape its answers carry. Anything else reads as no time.
fn epoch_seconds(text: &str) -> Option<Timestamp> {
    let [year, month, day, hour, minute, second] = clock_parts(text)?;
    let shaped = year != 0
        && (1..=12).contains(&month)
        && (1..=days_in_month(year, month)).contains(&day)
        && hour <= 23;
    if !shaped {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(Timestamp::from_secs(
        days * 86_400 + hour * 3_600 + minute * 60 + second,
    ))
}

// The six numbers of `YYYY-MM-DDTHH:MM:SSZ`; any other shape is none.
fn clock_parts(text: &str) -> Option<[u64; 6]> {
    let (date, rest) = text.split_once('T')?;
    let clock = rest.strip_suffix('Z')?;
    let parts: Vec<&str> = date.split('-').chain(clock.split(':')).collect();
    let shaped = parts.len() == 6 && parts[0].len() == 4 && parts[1..].iter().all(|p| p.len() == 2);
    if !shaped {
        return None;
    }
    let mut numbers = parts.iter().map(|part| part.parse::<u64>().ok());
    Some([
        numbers.next()??,
        numbers.next()??,
        numbers.next()??,
        numbers.next()??,
        numbers.next()??,
        numbers.next()??,
    ])
}

fn days_in_month(year: u64, month: u64) -> u64 {
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => match leap {
            true => 29,
            false => 28,
        },
    }
}

// Days from 1970-01-01 to `year-month-day`, counting March first so a leap
// day lands at the end of the year.
fn days_from_civil(year: u64, month: u64, day: u64) -> u64 {
    let (year, month) = match month <= 2 {
        true => (year - 1, month + 9),
        false => (year, month - 3),
    };
    let era = year / 400;
    let within = year % 400;
    let day_of_year = (153 * month + 2) / 5 + day - 1;
    let day_of_era = within * 365 + within / 4 - within / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_asks_for_the_repo_alone_and_for_what_the_grant_allows() {
        let koji = ForgeSlug::try_from("shep-pm/koji".to_owned()).unwrap();
        assert_eq!(
            token_request(&koji, Grant::Kelpie),
            r#"{"permissions":{"issues":"write","pull_requests":"write"},"repositories":["koji"]}"#
        );
        assert_eq!(
            token_request(&koji, Grant::Worker { workflows: false }),
            r#"{"permissions":{"contents":"write"},"repositories":["koji"]}"#
        );
        assert_eq!(
            token_request(&koji, Grant::Worker { workflows: true }),
            r#"{"permissions":{"contents":"write","workflows":"write"},"repositories":["koji"]}"#
        );
    }

    #[test]
    fn an_answer_s_time_is_read_as_whole_seconds_and_a_stranger_is_not() {
        let at = |text| epoch_seconds(text).map(|at| at.secs());
        assert_eq!(at("2026-11-10T17:00:00Z"), Some(1_794_330_000));
        assert_eq!(at("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(at("2024-02-29T23:59:59Z"), Some(1_709_251_199));
        for bad in [
            "2026-11-10 17:00:00",
            "2026-11-10T17:00:00",
            "2026-11-10T17:00:00+00:00",
            "2026-02-30T00:00:00Z",
            "2026-11-10T24:00:00Z",
            "2026-11-1T17:00:00Z",
            "not a time",
        ] {
            assert_eq!(at(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_token_answer_without_a_token_or_a_time_is_unreadable_naming_it() {
        let err = parse_token(r#"{"token":"","expires_at":"2026-11-10T17:00:00Z"}"#).unwrap_err();
        assert_eq!(err, ApiError::Unreadable("token"), "{err}");
        let err = parse_token(r#"{"token":"ghs_x","expires_at":"later"}"#).unwrap_err();
        assert_eq!(err, ApiError::Unreadable("expires_at"), "{err}");
    }

    #[test]
    fn a_key_s_text_never_shows_in_debug() {
        let pem = Pem::new("-----BEGIN RSA PRIVATE KEY-----\ns3cr3t\n".to_owned());
        assert_eq!(format!("{pem:?}"), "Pem(..)");
    }
}
