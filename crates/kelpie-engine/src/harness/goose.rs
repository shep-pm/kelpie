//! goose, which speaks ACP itself (`goose acp`)
//!
//! goose is native to ACP (ADR 0007), so kelpie runs the person's own `goose`
//! with its `acp` subcommand and installs no bridge. goose reaches local models
//! as readily as hosted ones, which is what makes it Phase 1's second harness
//! (ADR 0006).
//!
//! Each session gets `<state_dir>/goose/` as its goose root, so the person's
//! own goose config, `~/.goose`, stays out until they turn it on (ADR 0006).
//! The provider is named in the environment; the model and the effort are config
//! options, as they are for every harness. goose answers a resume with
//! `session/load` only, which [`crate::session`] handles for any harness that
//! cannot resume (`docs/acp.md`).
//!
//! goose puts each shell command it runs in a process group of its own, so
//! stopping a session stops those through the sweep for the run's mark, not
//! through the harness's group (ADR 0001, ADR 0007).
//!
//! A sandboxed session (ADR 0005) lets through the host of the provider's
//! address, `localhost` included, and sets goose's `auto` mode, which approves
//! every call.

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::Path;

use crate::session::Harness;

use super::{HarnessError, install_folders, on_path};

/// The command kelpie looks for on `PATH`, and runs
const CMD: &str = "goose";

/// The subcommand that makes goose speak ACP on its stdin and stdout
const ACP: &str = "acp";

/// The harness, as kelpie names it to the person
const NAME: &str = "goose";

/// The variable that gives the session a goose root of its own
const PATH_ROOT_ENV: &str = "GOOSE_PATH_ROOT";

/// The variable that names the provider the session runs on
const PROVIDER_ENV: &str = "GOOSE_PROVIDER";

/// The goose `mode` that approves every tool call, which a sandboxed session
/// sets: the sandbox is what confines it
const UNPROMPTED: &str = "auto";

/// The folder inside a session's state dir that goose keeps its files in
const CONFIG_FOLDER: &str = "goose";

/// The provider a goose session runs on, and where it answers
///
/// `name` is goose's own name for the provider, such as `ollama`. Kelpie names
/// no provider of its own (ADR 0006): this is the person's choice, held in the
/// session until the settings page can hold it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    /// The provider, as goose names it
    pub name: String,

    /// Where the provider answers, such as `http://127.0.0.1:11434`
    ///
    /// `None` leaves goose to its own config, and to what the person's own
    /// environment already says.
    pub host: Option<String>,
}

/// The [`Harness`] that runs the person's own goose over ACP
///
/// `state_dir` is the folder the caller keeps for this session, such as
/// `<data_dir>/projects/<id>/pm/`; goose gets `<state_dir>/goose/`.
/// `provider` is where the person's model comes from.
///
/// # Errors
///
/// [`HarnessError::NotInstalled`] when `PATH` holds no `goose`.
pub fn harness(state_dir: &Path, provider: Provider) -> Result<Harness, HarnessError> {
    let path = env::var_os("PATH").unwrap_or_default();
    harness_on_path(state_dir, &provider, &path)
}

/// [`harness`], searching `path` for the person's goose instead of this
/// process's `PATH`
pub(super) fn harness_on_path(
    state_dir: &Path,
    provider: &Provider,
    path: &OsStr,
) -> Result<Harness, HarnessError> {
    let goose = on_path(CMD, path).ok_or_else(|| HarnessError::NotInstalled {
        harness: NAME.to_owned(),
        cmd: CMD.to_owned(),
    })?;
    let mut env = vec![
        (PATH_ROOT_ENV.into(), state_dir.join(CONFIG_FOLDER).into()),
        (PROVIDER_ENV.into(), provider.name.clone().into()),
    ];
    if let Some(host) = &provider.host {
        env.push((host_env(&provider.name), host.clone().into()));
    }
    let installs = install_folders(&goose);
    Ok(Harness {
        program: goose,
        args: vec![ACP.into()],
        env,
        // goose asks for nothing of kelpie's on `session/new`
        new_session_meta: None,
        // kelpie reads no goose notification beyond ACP's own
        extensions: Vec::new(),
        hosts: provider
            .host
            .iter()
            .filter_map(|url| host_of(url))
            .collect(),
        installs,
        unprompted: Some(UNPROMPTED.to_owned()),
    })
}

/// The host in a provider's address, such as `127.0.0.1` in
/// `http://127.0.0.1:11434`
///
/// The sandbox lets this host through, so the session reaches the model the
/// person chose and nothing else (ADR 0005). `None` for an address with no host
/// in it. A provider with no address of its own has no host here, and a sandboxed
/// session on it reaches only what the project's allowlist names.
fn host_of(address: &str) -> Option<String> {
    let rest = address.split_once("://").map_or(address, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    // An IPv6 literal keeps its brackets, as the sandbox's allowlist spells it
    let host = match host.strip_prefix('[') {
        Some(v6) => &host[..=v6.find(']')? + 1],
        None => host.split(':').next()?,
    };
    (!host.is_empty()).then(|| host.to_owned())
}

/// The variable provider `name` reads its host from
///
/// goose names its own variables, and kelpie has checked this for `ollama`,
/// which reads `OLLAMA_HOST`. For any other provider kelpie uses the same shape,
/// `<PROVIDER>_HOST`, which is how goose names the host of its local providers;
/// kelpie has not checked it beyond `ollama`. A provider whose endpoint variable
/// is named otherwise needs its own entry here, and until it has one the
/// variable kelpie sets is one goose ignores. What the person's own environment
/// already holds reaches the session either way.
fn host_env(name: &str) -> OsString {
    format!("{}_HOST", name.to_uppercase()).into()
}
