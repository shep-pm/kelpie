//! The App's calls over `curl`, and its signer over `openssl`
//!
//! Each call reaches curl as a config on its stdin, as SK's webhook posts do, so
//! no process listing shows a JWT. `openssl` reads the key from its file and the
//! JWT's header and claims from its stdin, so signing never loads the key into
//! kelpie. Both run through [`ProcessTree::output_with_input`] (ADR 0001), so a
//! quit stops a call mid-flight and nothing kelpie starts escapes the tree.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use super::api::{self, ApiError, Cooldown, GithubApi, Grant, Installation, IssuedToken, Signer};
use super::slug::ForgeSlug;
use super::tokens::Jwt;
use crate::tree::ProcessTree;

const API: &str = "https://api.github.com";

/// Seconds curl gets to connect, and to finish the whole call
const CONNECT_TIMEOUT: u32 = 5;
const MAX_TIME: u32 = 30;

/// What curl prints after the body: the two headers a rate limit answers with,
/// on a line of their own, then the status, which [`run`] takes
const WRITE_OUT: &str = "\n%header{retry-after}|%header{x-ratelimit-reset}\n%{http_code}";

/// The App's calls, made with the system's `curl` through the process tree
#[derive(Debug, Clone)]
pub struct CurlGithub {
    tree: Arc<ProcessTree>,
}

impl CurlGithub {
    /// The calls, started through `tree`
    pub fn new(tree: &Arc<ProcessTree>) -> Self {
        Self {
            tree: Arc::clone(tree),
        }
    }
}

impl GithubApi for CurlGithub {
    fn installation(&self, jwt: &Jwt, repo: &ForgeSlug) -> Result<Option<Installation>, ApiError> {
        let url = format!("{API}/repos/{repo}/installation");
        match call(&self.tree, &url, Some(jwt.expose()), None)? {
            (body, 200, _) => api::parse_installation(&body).map(Some),
            (_, 404, _) => Ok(None),
            (body, status, cooldown) => Err(api::refusal(status, &body, cooldown)),
        }
    }

    fn access_token(
        &self,
        jwt: &Jwt,
        installation: u64,
        repo: &ForgeSlug,
        grant: Grant,
    ) -> Result<IssuedToken, ApiError> {
        let url = format!("{API}/app/installations/{installation}/access_tokens");
        let body = api::token_request(repo, grant);
        match call(&self.tree, &url, Some(jwt.expose()), Some(&body))? {
            (body, 201, _) => api::parse_token(&body),
            (body, status, cooldown) => Err(api::refusal(status, &body, cooldown)),
        }
    }
}

// One call to `url`, a `POST` of `post` when there is one, and what it answered
// with its status and when its headers say to ask again. A redirect is never
// followed, so a JWT goes nowhere but GitHub's API.
fn call(
    tree: &ProcessTree,
    url: &str,
    bearer: Option<&str>,
    post: Option<&str>,
) -> Result<(String, u16, Cooldown), ApiError> {
    let mut lines = vec![
        ("url", url.to_owned()),
        ("proto", "=https".to_owned()),
        ("connect-timeout", CONNECT_TIMEOUT.to_string()),
        ("max-time", MAX_TIME.to_string()),
        ("header", "Accept: application/vnd.github+json".to_owned()),
        ("header", "X-GitHub-Api-Version: 2022-11-28".to_owned()),
        ("user-agent", "kelpie".to_owned()),
        ("write-out", WRITE_OUT.to_owned()),
    ];
    if let Some(bearer) = bearer {
        lines.push(("header", format!("Authorization: Bearer {bearer}")));
    }
    if let Some(body) = post {
        lines.push(("header", "Content-Type: application/json".to_owned()));
        lines.push(("data-raw", body.to_owned()));
    }
    let (printed, status) = run(tree, &render(&lines))?;
    let (body, cooldown) = split_cooldown(&printed);
    Ok((body.to_owned(), status, cooldown))
}

// `printed` without its last line, which is `WRITE_OUT`'s headers, and what
// those say. A curl too old to know `%header` prints the line as it was
// written, which reads as no cooldown.
fn split_cooldown(printed: &str) -> (&str, Cooldown) {
    let (body, headers) = printed.rsplit_once('\n').unwrap_or(("", printed));
    let (retry_after, reset) = headers.split_once('|').unwrap_or_default();
    (body, Cooldown::read(retry_after, reset))
}

// Runs curl on `config`, whose `write-out` prints the HTTP status last, and
// returns what it printed before the status, and the status. Curl's own
// messages are dropped: an error names only curl's exit or the status.
fn run(tree: &ProcessTree, config: &str) -> Result<(String, u16), ApiError> {
    let mut command = Command::new("curl");
    command.args(["--config", "-"]);
    let output = tree
        .output_with_input(&mut command, config.as_bytes())
        .map_err(|e| ApiError::Unreachable(format!("cannot run curl: {e}")))?;
    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        return Err(ApiError::Unreachable(format!("curl exited {code}")));
    }
    let printed = String::from_utf8_lossy(&output.stdout).into_owned();
    let (printed, status) = printed.rsplit_once('\n').unwrap_or(("", &printed));
    let status = status
        .trim()
        .parse()
        .map_err(|_| ApiError::Unreachable("curl printed no status".to_owned()))?;
    Ok((printed.to_owned(), status))
}

/// curl's config: one `option = "value"` per line, silent first
fn render(lines: &[(&str, String)]) -> String {
    let options = lines
        .iter()
        .map(|(option, value)| format!("{option} = \"{}\"\n", quote(value)));
    std::iter::once("silent\n".to_owned())
        .chain(options)
        .collect()
}

// The escapes a quoted curl config value reads.
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// Signs with the system's `openssl`, which reads the key from its file and the
/// JWT's header and claims from its stdin
#[derive(Debug, Clone)]
pub struct Openssl {
    tree: Arc<ProcessTree>,
}

impl Openssl {
    /// The signer, started through `tree`
    pub fn new(tree: &Arc<ProcessTree>) -> Self {
        Self {
            tree: Arc::clone(tree),
        }
    }
}

impl Signer for Openssl {
    fn sign(&self, key: &Path, input: &[u8]) -> Result<Vec<u8>, String> {
        let mut command = Command::new("openssl");
        command.args(["dgst", "-sha256", "-sign"]).arg(key);
        let output = self
            .tree
            .output_with_input(&mut command, input)
            .map_err(|e| format!("cannot run openssl: {e}"))?;
        let failed = !output.status.success() || output.stdout.is_empty();
        match failed {
            false => Ok(output.stdout),
            true => Err(format!(
                "openssl could not sign with {}: {}",
                key.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::github::Timestamp;
    use crate::worktree::tests::{Dir, process_tree};

    #[test]
    fn the_headers_a_rate_limit_answers_with_are_read_after_the_body() {
        let (body, cooldown) = split_cooldown("{\"message\":\"slow\"}\n30|");
        assert_eq!(body, "{\"message\":\"slow\"}");
        assert_eq!(cooldown.retry_after, Some(30));
        assert_eq!(cooldown.reset, None);

        let (_, cooldown) = split_cooldown("{}\n|1790003600");
        assert_eq!(cooldown.retry_after, None);
        assert_eq!(cooldown.reset, Some(Timestamp::from_secs(1_790_003_600)));
    }

    #[test]
    fn a_body_over_several_lines_keeps_them_and_no_headers_read_as_no_cooldown() {
        let (body, cooldown) = split_cooldown("a\nb\n|");
        assert_eq!(body, "a\nb");
        assert_eq!(cooldown, Cooldown::default());

        // A curl that does not know `%header` prints the line as written.
        let (body, cooldown) =
            split_cooldown("{}\n%header{retry-after}|%header{x-ratelimit-reset}");
        assert_eq!(body, "{}");
        assert_eq!(cooldown, Cooldown::default());
    }

    #[test]
    fn an_answer_with_no_body_has_no_cooldown_either() {
        let (body, cooldown) = split_cooldown("\n|");
        assert_eq!((body, cooldown), ("", Cooldown::default()));
    }

    #[test]
    fn a_config_quotes_every_value_so_none_of_it_is_an_option_of_its_own() {
        let lines = [
            ("url", "http://127.0.0.1/x".to_owned()),
            ("data-raw", "{\"a\":\"b\\n\"}".to_owned()),
        ];
        assert_eq!(
            render(&lines),
            "silent\nurl = \"http://127.0.0.1/x\"\ndata-raw = \"{\\\"a\\\":\\\"b\\\\n\\\"}\"\n"
        );
    }

    // The one route a call takes: curl on its stdin, in the tree, with the
    // status on its last line. A file transfer answers `000`, which is enough
    // to show the body and the status come apart.
    #[test]
    fn a_call_runs_curl_on_its_stdin_through_the_tree() {
        let dir = Dir::new("curl-call");
        let root = dir.path().canonicalize().unwrap();
        let tree = process_tree(&root);
        fs::write(root.join("body"), "hello").unwrap();

        let config = render(&[
            ("url", format!("file://{}/body", root.display())),
            ("proto", "=file".to_owned()),
            ("header", "Authorization: Bearer eyJ.abc".to_owned()),
            ("write-out", "\n%{http_code}".to_owned()),
        ]);
        let (body, status) = run(&tree, &config).expect("curl did not answer");

        assert_eq!((body.as_str(), status), ("hello", 0));
    }

    #[test]
    fn a_call_that_cannot_be_made_names_curl_and_nothing_of_the_call() {
        let dir = Dir::new("curl-unreachable");
        let tree = process_tree(&dir.path().canonicalize().unwrap());
        let config = render(&[
            ("url", "http://127.0.0.1:1/".to_owned()),
            ("proto", "=http".to_owned()),
            ("connect-timeout", "2".to_owned()),
            ("header", "Authorization: Bearer eyJ.s3cr3t".to_owned()),
            ("write-out", "\n%{http_code}".to_owned()),
        ]);

        let err = run(&tree, &config).unwrap_err();

        assert!(matches!(err, ApiError::Unreachable(_)), "{err}");
        assert!(!err.to_string().contains("s3cr3t"), "{err}");
        assert!(!err.to_string().contains("127.0.0.1"), "{err}");
    }

    // A JWT is never a process argument: it is on curl's stdin, so a listing
    // of the call shows the URL alone.
    #[test]
    fn a_jwt_never_reaches_the_command_line() {
        let lines = vec![("header", "Authorization: Bearer eyJ.s3cr3t".to_owned())];
        let config = render(&lines);
        assert!(config.contains("s3cr3t"), "the JWT belongs on the config");
        let args: Vec<String> = Command::new("curl")
            .args(["--config", "-"])
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["--config", "-"]);
        assert!(!args.iter().any(|arg| arg.contains("s3cr3t")));
    }
}
