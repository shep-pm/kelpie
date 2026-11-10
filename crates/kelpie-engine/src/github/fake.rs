//! A stand-in GitHub for kelpie's App, and a signer that needs no key
//!
//! The stand-in answers the two calls a token is minted with, records what it
//! was asked, and can hold a permission back or grant one it was never asked
//! for, which is how the tests show a worker is handed a token for pushing and
//! nothing more.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::api::{self, ApiError, GithubApi, Grant, Installation, IssuedToken, Signer};
use super::slug::ForgeSlug;
use super::tokens::{AppTokens, InstallationToken, Jwt};
use super::{App, Apps, Clock, Pem, Timestamp};

/// A key no test would print by chance, so finding it anywhere else is a leak
pub const PEM: &str =
    "-----BEGIN RSA PRIVATE KEY-----\nkelpie-test-app-key-s3cr3t\n-----END RSA PRIVATE KEY-----\n";

/// What the stand-in was asked, in order
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// An installation lookup, and the repo it was asked about
    Installation(String),
    /// An installation's token, and the body that asked for it
    AccessToken(u64, String),
}

#[derive(Debug, Default)]
struct Hub {
    installed: BTreeMap<String, u64>,
    // Whether the installations hold the `workflows` permission
    workflows: bool,
    // What the next calls answer in place of their success, in turn
    failures: VecDeque<ApiError>,
    asked: Vec<Asked>,
    jwts: Vec<String>,
    minted: u32,
    // A permission GitHub grants a worker's token beyond the one asked for
    extra_grant: Option<(String, String)>,
}

/// GitHub as kelpie's App sees it: one App owned by `owner`, installed on the
/// repos a test installs it on, and each token it mints lasts an hour from the
/// clock's now
#[derive(Debug, Clone)]
pub struct FakeGithub {
    hub: Arc<Mutex<Hub>>,
    clock: FakeClock,
    owner: String,
}

impl FakeGithub {
    /// How long a token it mints lasts, as GitHub's do
    pub const LIFETIME: u64 = 3600;

    /// A stand-in GitHub for `owner`, with its tokens' lifetimes on `clock`
    pub fn new(clock: FakeClock, owner: &str) -> Self {
        Self {
            hub: Arc::default(),
            clock,
            owner: owner.to_owned(),
        }
    }

    /// The App the stand-in stands for
    pub fn app(&self) -> App {
        App {
            id: 42,
            slug: format!("kelpie-{}", self.owner.to_ascii_lowercase()),
            client_id: "Iv23liTEST".to_owned(),
            owner: self.owner.clone(),
        }
    }

    /// Installs the App on `repo` as installation `id`
    pub fn install(&self, repo: &str, id: u64) {
        let repo = repo.to_ascii_lowercase();
        self.hub.lock().unwrap().installed.insert(repo, id);
    }

    /// Registers the App in `kelpie_home` as keeping one writes it, and installs
    /// it on `repo`
    pub fn registered(&self, kelpie_home: &Path, repo: &str) {
        Apps::under(kelpie_home)
            .save(&self.app(), &Pem::new(PEM.to_owned()))
            .unwrap();
        self.install(repo, 7);
    }

    /// Takes the App off `repo`, as uninstalling it there does
    pub fn uninstall(&self, repo: &str) {
        let repo = repo.to_ascii_lowercase();
        self.hub.lock().unwrap().installed.remove(&repo);
    }

    /// Gives the App the `workflows` permission, accepted on every installation
    pub fn hold_workflows(&self) {
        self.hub.lock().unwrap().workflows = true;
    }

    /// Makes the next call answer `error`, after any failures already set
    pub fn fail_next(&self, error: ApiError) {
        self.hub.lock().unwrap().failures.push_back(error);
    }

    /// Makes GitHub grant every worker's token `permission` at `level` besides
    /// what was asked for
    pub fn grants_too(&self, permission: &str, level: &str) {
        let grant = (permission.to_owned(), level.to_owned());
        self.hub.lock().unwrap().extra_grant = Some(grant);
    }

    /// What it was asked, in order
    pub fn asked(&self) -> Vec<Asked> {
        self.hub.lock().unwrap().asked.clone()
    }

    /// Every JWT a call was made with
    pub fn jwts(&self) -> Vec<String> {
        self.hub.lock().unwrap().jwts.clone()
    }

    /// kelpie's App tokens over this stand-in and [`FakeSigner`], kept in `kelpie_home`
    pub fn tokens(&self, kelpie_home: &Path) -> AppTokens {
        AppTokens::new(
            Apps::under(kelpie_home),
            Box::new(self.clone()),
            Box::new(FakeSigner),
            Box::new(self.clock.clone()),
        )
    }
}

impl GithubApi for FakeGithub {
    fn installation(&self, jwt: &Jwt, repo: &ForgeSlug) -> Result<Option<Installation>, ApiError> {
        let mut hub = self.hub.lock().unwrap();
        hub.asked
            .push(Asked::Installation(repo.as_str().to_owned()));
        hub.jwts.push(jwt.expose().to_owned());
        if let Some(error) = hub.failures.pop_front() {
            return Err(error);
        }
        let repo = repo.as_str().to_ascii_lowercase();
        let workflows = hub.workflows;
        Ok(hub
            .installed
            .get(&repo)
            .map(|&id| Installation { id, workflows }))
    }

    fn access_token(
        &self,
        jwt: &Jwt,
        installation: u64,
        repo: &ForgeSlug,
        grant: Grant,
    ) -> Result<IssuedToken, ApiError> {
        let body = api::token_request(repo, grant);
        let mut hub = self.hub.lock().unwrap();
        hub.asked
            .push(Asked::AccessToken(installation, body.clone()));
        hub.jwts.push(jwt.expose().to_owned());
        if let Some(error) = hub.failures.pop_front() {
            return Err(error);
        }
        hub.minted += 1;
        let ask = |pairs: &[(&str, &str)]| {
            (pairs.iter())
                .map(|&(name, level)| (name.to_owned(), level.to_owned()))
                .collect::<BTreeMap<_, _>>()
        };
        let mut permissions = match grant {
            Grant::Kelpie => ask(&[
                ("issues", "write"),
                ("metadata", "read"),
                ("pull_requests", "write"),
            ]),
            Grant::Worker { workflows: false } => {
                ask(&[("contents", "write"), ("metadata", "read")])
            }
            Grant::Worker { workflows: true } => ask(&[
                ("contents", "write"),
                ("metadata", "read"),
                ("workflows", "write"),
            ]),
        };
        let extra = (hub.extra_grant.clone()).filter(|_| matches!(grant, Grant::Worker { .. }));
        if let Some((name, level)) = extra {
            permissions.insert(name, level);
        }
        Ok(IssuedToken {
            permissions,
            token: InstallationToken::new(format!("ghs_test{}", hub.minted)),
            expires_at: Timestamp::from_secs(self.clock.now().secs() + Self::LIFETIME),
        })
    }
}

/// Signs anything with one fixed signature
#[derive(Debug, Clone, Copy, Default)]
pub struct FakeSigner;

impl FakeSigner {
    /// What every signature is
    pub const SIGNATURE: &[u8] = b"signed";
}

impl Signer for FakeSigner {
    fn sign(&self, _key: &Path, _input: &[u8]) -> Result<Vec<u8>, String> {
        Ok(Self::SIGNATURE.to_vec())
    }
}

/// A clock that stands where a test puts it, so a token's life is tested without waiting
#[derive(Debug, Clone)]
pub struct FakeClock(Arc<AtomicU64>);

impl FakeClock {
    /// A clock that stands at `seconds` past the epoch
    pub fn at(seconds: u64) -> Self {
        Self(Arc::new(AtomicU64::new(seconds)))
    }

    /// Moves the clock on by `seconds`
    pub fn advance(&self, seconds: u64) {
        self.0.fetch_add(seconds, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_secs(self.0.load(Ordering::SeqCst))
    }
}
