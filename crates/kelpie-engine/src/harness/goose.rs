//! goose, which speaks ACP itself (`goose acp`)
//!
//! goose is native to ACP (ADR 0007), so kelpie runs the person's own `goose`
//! with its `acp` subcommand and installs no bridge. goose reaches local models
//! as readily as hosted ones, which is what makes it Phase 1's second harness
//! (ADR 0006).
//!
//! Each session gets `<state_dir>/goose/` as its goose root, and goose keeps
//! its config one folder under that root, in `<state_dir>/goose/config/`, so the
//! person's own goose config stays out until they turn it on (ADR 0006). With
//! the setting on, their own `config.yaml` is linked into that folder;
//! `secrets.yaml`, which holds their model keys, never is.
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
//! address, or goose's default host for the provider when it has none,
//! `localhost` included, and sets goose's `auto` mode, which approves every
//! call.

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use agent_client_protocol::v1::schema::ToolCallUpdate;

use crate::permission::{Call, ReadCall, read_acp};
use crate::sandbox::fence::OwnFiles;
use crate::session::Harness;

use super::global::{self, Entry};
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

/// The goose `mode` that asks before each tool call, which a judged session
/// sets so every call reaches the permission handler
const PROMPTING: &str = "approve";

/// The field of goose's tool input that names the file it writes
const PATH_FIELDS: [&str; 1] = ["path"];

/// What a permission request's tool call does, in goose's words
///
/// goose's file tool takes the file as `path`, and its shell tool a `command`.
/// goose also gives each call's locations, which this falls back on.
#[must_use]
pub fn read_call(tool_call: &ToolCallUpdate) -> Call {
    // parity: harness.permission
    read_acp(tool_call, &PATH_FIELDS)
}

/// The folder inside a session's state dir that kelpie gives goose as its root
const CONFIG_FOLDER: &str = "goose";

/// The folder inside a goose root that goose keeps its config in
///
/// goose's path code names its root with `GOOSE_PATH_ROOT` and joins `config` to
/// it for its config folder, with its `data` and `state` folders beside it, so
/// `config.yaml` is one folder under the root and never in the root itself.
///
/// The goose on `PATH` (block-goose-cli 1.57.0) says so in three ways: it carries
/// `crates/goose/src/config/paths.rs`; it exports `goose::config::paths::Paths`
/// with `config_dir`, `data_dir`, `state_dir`, `in_data_dir` and `in_state_dir`
/// beside a single `get_dir`, which is one base with its folders under it; and
/// its read-only data holds `config` and `state` beside `GOOSE_PATH_ROOT` and the
/// `goose requires a home dir` of the branch taken when it is unset. The path is
/// built one join at a time, so `config/config` is never a literal to search for.
const ROOT_CONFIG: &str = "config";

/// The folder goose keeps its config in when the person sets no root of their own
///
/// `~/.config/goose`, which is where the flat `~/.config/goose/config.yaml` of
/// goose's own docs sits, and where its prompt template sends a person who wants
/// to override one (`~/.config/goose/prompts/compaction_summary.md`) in the macOS
/// build of the goose on `PATH`: goose lays out `config`, `data` and `state` under
/// `~/.config`, `~/.local/share` and `~/.local/state` on every platform, which is
/// the set of folders [`OWN_FILES`] closes as its login.
const GLOBAL_CONFIG: &str = ".config/goose";

/// Each entry of the person's goose config folder a session reads when they turn it on
///
/// goose's own config, which holds its extensions and its default provider. The
/// secrets goose keeps beside it are not an entry, and the sandbox keeps them
/// closed (ADR 0005).
const GLOBAL_ENTRIES: [Entry; 1] = ["config.yaml"];

/// What the sandbox's fence keeps a worker from changing of goose's, and from
/// reading of its login
///
/// goose also loads `.goosehints` from the working folder. It is not listed
/// here: kelpie sets `CONTEXT_FILE_NAMES` so goose never reads it.
pub const OWN_FILES: OwnFiles = OwnFiles {
    harness: NAME,
    folders: &[".goose"],
    // `settings.local.json` and `settings.json` there can disable a plugin by name
    nested: &[[".config", "goose"]],
    root_files: &[],
    // Its config, secrets and session database sit in four places, by platform
    credentials: &[
        "~/.config/goose/**",
        "~/.local/share/goose/**",
        "~/.local/state/goose/**",
        "~/Library/Application Support/Block/goose/**",
    ],
    program: CMD,
};

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
/// `<data_dir>/projects/<id>/pm/`; goose gets `<state_dir>/goose/` as its root,
/// and reads its config from `<state_dir>/goose/config/`.
/// `provider` is where the person's model comes from.
/// `harness_global_folder` is the project's resolved setting: on links the
/// person's own `config.yaml` into that config folder, off takes back the links a
/// session made with it on.
///
/// # Errors
///
/// [`HarnessError::NotInstalled`] when `PATH` holds no `goose`, and
/// [`HarnessError::Global`] when a link could not be made or removed.
pub fn harness(
    state_dir: &Path,
    provider: Provider,
    harness_global_folder: bool,
) -> Result<Harness, HarnessError> {
    let path = env::var_os("PATH").unwrap_or_default();
    let folder = global_folder(
        env::var_os(PATH_ROOT_ENV).as_deref(),
        global::home().as_deref(),
    );
    harness_on_path(
        state_dir,
        &provider,
        &path,
        folder.as_deref(),
        harness_global_folder,
    )
}

/// [`harness`], searching `path` for the person's goose instead of this
/// process's `PATH`, and reading their `config.yaml` from their own config folder
/// `global` when it has one
pub(super) fn harness_on_path(
    state_dir: &Path,
    provider: &Provider,
    path: &OsStr,
    global: Option<&Path>,
    harness_global_folder: bool,
) -> Result<Harness, HarnessError> {
    let goose = on_path(CMD, path).ok_or_else(|| HarnessError::NotInstalled {
        harness: NAME.to_owned(),
        cmd: CMD.to_owned(),
    })?;
    // The root goose is told to use, and the config folder goose reads inside it.
    // A link belongs in the second: goose never looks in the root itself.
    let (root, config_dir) = (
        state_dir.join(CONFIG_FOLDER),
        state_dir.join(CONFIG_FOLDER).join(ROOT_CONFIG),
    );
    let mut env = vec![
        (PATH_ROOT_ENV.into(), root.clone().into()),
        (PROVIDER_ENV.into(), provider.name.clone().into()),
    ];
    if let Some(host) = &provider.host {
        env.push((host_env(&provider.name), host.clone().into()));
    }
    let installs = install_folders(&goose, path);
    let global_reads = match (global, harness_global_folder) {
        (Some(folder), true) => global::link(folder, &config_dir, &GLOBAL_ENTRIES)?,
        // Off, or no folder to name: take back every link a past start made, from
        // whichever folder it linked
        _ => {
            global::unlink(&config_dir, &GLOBAL_ENTRIES)?;
            Vec::new()
        }
    };
    Ok(Harness {
        program: goose,
        args: vec![ACP.into()],
        env,
        // goose asks for nothing of kelpie's on `session/new`
        new_session_meta: None,
        // kelpie reads no goose notification beyond ACP's own
        extensions: Vec::new(),
        hosts: hosts_of(provider),
        installs,
        global_reads,
        unprompted: Some(UNPROMPTED.to_owned()),
        prompting: Some(PROMPTING.to_owned()),
        read_call: ReadCall::new(read_call),
        // goose speaks ACP itself, so no bridge says anything about its version
        outdated: None,
    })
}

/// The folder the person's own goose reads its config from
///
/// `root` is what their `GOOSE_PATH_ROOT` says, `home` what their OS does. goose
/// keeps its config one folder under a root it names, and directly in the `goose`
/// folder under the OS's config folder when it names no root. `None` when neither
/// can be named.
pub(super) fn global_folder(root: Option<&OsStr>, home: Option<&Path>) -> Option<PathBuf> {
    match root.filter(|root| !root.is_empty()) {
        Some(root) => Some(Path::new(root).join(ROOT_CONFIG)),
        None => home.map(|home| home.join(GLOBAL_CONFIG)),
    }
}

/// The hosts a sandboxed session on `provider` may reach
///
/// The host of the address the person gave, or else the host goose's own
/// default for the provider answers on, so a session on `anthropic` or `openai`
/// with no address still reaches its model. A provider that is in neither place
/// has no host here, and a sandboxed session on it reaches only what the
/// project's allowlist names.
fn hosts_of(provider: &Provider) -> Vec<String> {
    match &provider.host {
        Some(address) => host_of(address).into_iter().collect(),
        None => default_host(&provider.name)
            .map(str::to_owned)
            .into_iter()
            .collect(),
    }
}

/// The host goose's own default for a provider answers on, for the providers
/// whose default is one host
///
/// Kelpie names no provider of its own (ADR 0006): this is only where goose
/// sends a session that gave no address, so the sandbox lets that through.
fn default_host(provider: &str) -> Option<&'static str> {
    Some(match provider {
        "anthropic" => "api.anthropic.com",
        "openai" => "api.openai.com",
        "google" => "generativelanguage.googleapis.com",
        "openrouter" => "openrouter.ai",
        "groq" => "api.groq.com",
        "xai" => "api.x.ai",
        "ollama" => "localhost",
        _ => return None,
    })
}

/// The host in a provider's address, such as `127.0.0.1` in
/// `http://127.0.0.1:11434`
///
/// The sandbox lets this host through, so the session reaches the model the
/// person chose and nothing else (ADR 0005). `None` for an address with no host
/// in it, and for a host the sandbox cannot name: crabpot takes a dotted name, an
/// IP literal or `localhost`, and refuses the whole policy over any other, so a
/// bare name such as `ollama` in `http://ollama:11434` is left out and the
/// session starts, reaching only what the project's allowlist names.
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
    sandbox_can_name(host).then(|| host.to_owned())
}

/// Whether crabpot's allowlist takes `host`: an IPv6 literal in brackets,
/// `localhost`, or a name with a dot inside it
fn sandbox_can_name(host: &str) -> bool {
    host.starts_with('[')
        || host == "localhost"
        || (host.contains('.') && !host.starts_with('.') && !host.ends_with('.'))
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
