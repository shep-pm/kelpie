//! Logins: the secret a harness runs with, kept in the OS keychain
//!
//! A login never sits in a settings file, a log, an event or an error message
//! (`docs/design/settings.md`). [`Store`] is the one place that holds it. The
//! app's commands reach [`Store::set`], [`Store::status`] and [`Store::clear`],
//! so the settings form can say whether a login is set and never what it is.
//! [`Store::get`] is the engine's own: a harness slice reads the login there
//! when it builds a session.
//!
//! Each harness has one keychain entry, under the service `kelpie` and the
//! harness's id as the user name.
//!
//! # Quick start
//!
//! ```
//! use kelpie_engine::login::Store;
//!
//! let store = Store::memory();
//! store.set("claude", "a-token")?;
//! assert!(store.status("claude")?);
//! store.clear("claude")?;
//! assert!(!store.status("claude")?);
//! # Ok::<(), kelpie_engine::login::LoginError>(())
//! ```

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

/// The keychain service every login is kept under
const SERVICE: &str = "kelpie";

/// Where the logins are kept
///
/// A clone shares the same logins.
#[derive(Clone)]
pub struct Store {
    backend: Backend,
}

/// What holds the logins
#[derive(Clone)]
enum Backend {
    /// The OS keychain
    Keychain,

    /// A map in memory, for tests
    Memory(Arc<Mutex<HashMap<String, String>>>),
}

impl Store {
    /// The OS keychain: Keychain Services, Credential Manager or the Secret Service
    #[must_use]
    pub const fn keychain() -> Self {
        Self {
            backend: Backend::Keychain,
        }
    }

    /// An empty store in memory, which keeps nothing past its last clone
    #[must_use]
    pub fn memory() -> Self {
        Self {
            backend: Backend::Memory(Arc::default()),
        }
    }

    /// Keep `value` as `harness`'s login, replacing any it had
    ///
    /// Surrounding whitespace is dropped, since a pasted token often carries a newline.
    ///
    /// # Errors
    ///
    /// [`LoginError::Blank`] when `value` holds nothing but whitespace, and
    /// [`LoginError::Keychain`] when the keychain cannot be written.
    pub fn set(&self, harness: &str, value: &str) -> Result<(), LoginError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(LoginError::Blank);
        }
        match &self.backend {
            Backend::Keychain => entry(harness)?
                .set_password(value)
                .map_err(LoginError::from),
            Backend::Memory(logins) => {
                lock(logins).insert(harness.to_owned(), value.to_owned());
                Ok(())
            }
        }
    }

    /// Whether `harness` has a login
    ///
    /// # Errors
    ///
    /// [`LoginError::Keychain`] when the keychain cannot be read.
    pub fn status(&self, harness: &str) -> Result<bool, LoginError> {
        self.get(harness).map(|login| login.is_some())
    }

    /// Forget `harness`'s login; a harness with none is left as it was
    ///
    /// # Errors
    ///
    /// [`LoginError::Keychain`] when the keychain cannot be written.
    pub fn clear(&self, harness: &str) -> Result<(), LoginError> {
        match &self.backend {
            Backend::Keychain => match entry(harness)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(error.into()),
            },
            Backend::Memory(logins) => {
                lock(logins).remove(harness);
                Ok(())
            }
        }
    }

    /// `harness`'s login, `None` when it has none
    ///
    /// For the engine's harness slices only: no command hands this value on.
    ///
    /// # Errors
    ///
    /// [`LoginError::Keychain`] when the keychain cannot be read.
    pub(crate) fn get(&self, harness: &str) -> Result<Option<String>, LoginError> {
        match &self.backend {
            Backend::Keychain => match entry(harness)?.get_password() {
                Ok(value) => Ok(Some(value)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(error) => Err(error.into()),
            },
            Backend::Memory(logins) => Ok(lock(logins).get(harness).cloned()),
        }
    }
}

/// A store holds secrets, so `Debug` names the backend and leaves the logins out
impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.backend {
            Backend::Keychain => "Store { keychain }",
            Backend::Memory(_) => "Store { memory }",
        })
    }
}

/// `harness`'s keychain entry
fn entry(harness: &str) -> Result<keyring::Entry, LoginError> {
    keyring::Entry::new(SERVICE, harness).map_err(LoginError::from)
}

/// The map behind a memory store; a panic elsewhere leaves the map whole, so a poisoned lock is used
fn lock(
    logins: &Mutex<HashMap<String, String>>,
) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
    logins.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Why a login could not be kept, read or forgotten
///
/// No variant carries a login's value, and none of their text names one.
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoginError {
    /// The value to keep was empty or only whitespace
    Blank,

    /// The OS keychain refused or could not be reached: locked, denied, or absent
    Keychain {
        /// What the keychain said, without any value
        reason: String,
    },
}

impl From<keyring::Error> for LoginError {
    fn from(error: keyring::Error) -> Self {
        // The data-carrying variants hold the stored bytes, so only the ones that describe the
        // platform's own failure pass their text on
        let reason = match error {
            keyring::Error::PlatformFailure(_)
            | keyring::Error::NoStorageAccess(_)
            | keyring::Error::NoDefaultStore => error.to_string(),
            keyring::Error::BadEncoding(_) | keyring::Error::BadDataFormat(..) => {
                "the stored login could not be read".to_owned()
            }
            _ => "the keychain could not complete the request".to_owned(),
        };
        Self::Keychain { reason }
    }
}

impl fmt::Display for LoginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blank => f.write_str("a login cannot be blank"),
            Self::Keychain { reason } => write!(f, "the OS keychain failed: {reason}"),
        }
    }
}

impl core::error::Error for LoginError {}

#[cfg(test)]
mod tests;
