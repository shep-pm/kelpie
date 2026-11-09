//! Claude Code, reached through its bridge from the ACP registry (ADR 0007)
//!
//! Kelpie installs the bridge the registry publishes, `claude-acp`, and never
//! Claude Code itself. The bridge drives the person's own `claude` through the
//! Claude Agent SDK, so kelpie points it there with `CLAUDE_CODE_EXECUTABLE` and
//! hands the session the person's own login (ADR 0006). The bridge's archive
//! carries a `claude` of its own, which [`crate::bridge::ensure`] leaves out.
//!
//! Each session gets `<state_dir>/claude/` as its config folder, so the person's
//! `~/.claude` stays out until they turn it on. Claude Code keeps its login per
//! config folder, which is why the session carries `CLAUDE_CODE_OAUTH_TOKEN`.
//!
//! The raw SDK messages asked for on `session/new` are what carry the split
//! between the one-hour and the five-minute prompt cache, which ACP's own usage
//! counts do not (`docs/acp.md`, Checked by hand).

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde_json::json;

use crate::bridge::{Download, Pin, Platform};
use crate::session::{Extension, Harness};

use super::{HarnessError, Login, on_path};

/// The registry id of the bridge kelpie installs
const ID: &str = "claude-acp";

/// The pinned bridge version, from the registry's `agent.json` on 2026-11-09
const VERSION: &str = "0.101.1";

/// Where the registry's prebuilt archives for one version hang off
const RELEASES: &str = "https://github.com/agentclientprotocol/claude-agent-acp/releases/download";

/// The command the bridge's archive holds, which is the one kelpie runs
const CMD: &str = "claude-agent-acp";

/// The harness, as kelpie names it to the person
const NAME: &str = "Claude Code";

/// The command kelpie looks for on `PATH`: the person's own Claude Code
const CLAUDE: &str = "claude";

/// The variable kelpie reads the login from, and sets for the bridge
pub const LOGIN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";

/// The variable that tells the bridge which `claude` to drive
const EXECUTABLE_ENV: &str = "CLAUDE_CODE_EXECUTABLE";

/// The variable that gives the session a config folder of its own
const CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";

/// The folder inside a session's state dir that Claude Code keeps its config in
const CONFIG_FOLDER: &str = "claude";

/// The command that makes the login token
const SETUP: &str = "claude setup-token";

/// The bridge archives the registry publishes for the pinned version
///
/// Copied from `claude-acp/agent.json` in `agentclientprotocol/registry`. The
/// registry names its command `./claude-agent-acp`; kelpie names the file it
/// unpacks, so the pin holds the bare name. Windows has no entry: kelpie has no
/// pinned archive for it yet.
static PLATFORMS: LazyLock<Vec<(Platform, Download)>> = LazyLock::new(|| {
    [
        (
            Platform::MacOsArm64,
            "claude-agent-acp-darwin-arm64.zip",
            "60acc44d1b9964456c22fb0f32ae12e25a7b2c37c4f98cb1b2345f690b6d1116",
        ),
        (
            Platform::MacOsX64,
            "claude-agent-acp-darwin-x64.zip",
            "bcf02862a1ecb5566b84da83777620fb26d48d5e046329a9177d55a4cc55d5f6",
        ),
        (
            Platform::LinuxX64,
            "claude-agent-acp-linux-x64.tar.gz",
            "5f06b504424ff6e4e9ae1cd12b3d483d4665eda742ba3ac91513f3cb45e8e5fc",
        ),
        (
            Platform::LinuxArm64,
            "claude-agent-acp-linux-arm64.tar.gz",
            "91889a2e6280d47293e599c95f4c2c61905b26c33d7e0e94a8719d2e08636222",
        ),
    ]
    .into_iter()
    .map(|(platform, archive, sha256)| {
        (
            platform,
            Download {
                url: format!("{RELEASES}/v{VERSION}/{archive}"),
                sha256: sha256.to_owned(),
                cmd: CMD.to_owned(),
            },
        )
    })
    .collect()
});

/// The pinned bridge, ready for [`bridge::ensure`]
static PIN: LazyLock<Pin<'static>> = LazyLock::new(|| Pin {
    id: ID,
    version: VERSION,
    platforms: PLATFORMS.as_slice(),
});

/// The `claude-acp` bridge kelpie installs: one pinned version, one archive per
/// platform kelpie ships on
#[must_use]
pub fn pin() -> &'static Pin<'static> {
    &PIN
}

/// The [`Harness`] that runs Claude Code through `bridge`
///
/// `bridge` is the installed bridge command, from [`crate::bridge::ensure`] with [`pin`].
/// `state_dir` is the folder the caller keeps for this session, such as
/// `<data_dir>/projects/<id>/pm/`; Claude Code gets `<state_dir>/claude/`.
/// `login` is the person's own Claude Code login.
///
/// # Errors
///
/// [`HarnessError::NotInstalled`] when `PATH` holds no `claude`, and
/// [`HarnessError::NoLogin`] when `login` holds no token.
pub fn harness(bridge: PathBuf, state_dir: &Path, login: Login) -> Result<Harness, HarnessError> {
    let path = env::var_os("PATH").unwrap_or_default();
    harness_on_path(bridge, state_dir, &login, &path)
}

/// [`harness`], searching `path` for the person's Claude Code instead of this
/// process's `PATH`
pub(super) fn harness_on_path(
    bridge: PathBuf,
    state_dir: &Path,
    login: &Login,
    path: &OsStr,
) -> Result<Harness, HarnessError> {
    let claude = on_path(CLAUDE, path).ok_or_else(|| HarnessError::NotInstalled {
        harness: NAME.to_owned(),
        cmd: CLAUDE.to_owned(),
    })?;
    let token = login.token().ok_or_else(|| HarnessError::NoLogin {
        harness: NAME.to_owned(),
        env: LOGIN_ENV.to_owned(),
        setup: SETUP.to_owned(),
    })?;
    Ok(Harness {
        program: bridge,
        args: Vec::new(),
        env: vec![
            (EXECUTABLE_ENV.into(), claude.into()),
            (CONFIG_DIR_ENV.into(), state_dir.join(CONFIG_FOLDER).into()),
            (LOGIN_ENV.into(), token.to_owned().into()),
        ],
        // The raw SDK messages are the only route to the cache lifetime (ADR 0007)
        new_session_meta: Some(json!({ "claudeCode": { "emitRawSDKMessages": true } })),
        extensions: vec![Extension::ClaudeSdkMessage],
    })
}

