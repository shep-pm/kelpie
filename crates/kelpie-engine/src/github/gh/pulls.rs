//! The pull request kelpie opens, edits and merges, as the maintainer
//!
//! The worker pushes its branch with a token that cannot open a pull request,
//! so it hands over its pull request's title and body in a file and kelpie opens
//! it as the login `gh` holds (`docs/design/tickets.md`, GitHub). A call as the
//! App stays behind: a bot's pull request loses the review bots. The merge and
//! the ready mark go through `gh pr`, since the REST merge does not join a merge
//! queue.

use std::num::NonZeroU64;

use serde::Deserialize;

use super::hold::Hold;
use crate::github::ForgeSlug;
use crate::local_paths::{LocalPaths, Surface};
use crate::ticket::{ForgeError, PullState, Pulls};

/// The draft pull request of one repo, opened through a [`Hold`]
#[derive(Debug)]
pub struct GithubPulls {
    hold: Hold,
    repo: ForgeSlug,
    paths: LocalPaths,
}

impl GithubPulls {
    /// Pull requests of `repo`, called through `hold`
    ///
    /// `paths` holds the project's folder and worktree, which no title or body
    /// may name.
    #[must_use]
    pub fn new(hold: Hold, repo: ForgeSlug, paths: LocalPaths) -> Self {
        Self { hold, repo, paths }
    }

    // A title or body that names this machine is refused before any call
    fn check(&self, title: &str, body: &str) -> Result<(), ForgeError> {
        let leaks = |text| self.paths.find(text, Surface::Prose).is_some();
        if leaks(title) || leaks(body) {
            return Err(ForgeError::LocalPath);
        }
        Ok(())
    }
}

impl Pulls for GithubPulls {
    fn create_draft(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<NonZeroU64, ForgeError> {
        self.check(title, body)?;
        let args = create_args(&self.repo, head, base, title, body);
        let out = self.hold.call(&as_strs(&args))?;
        let text = String::from_utf8_lossy(&out);
        text.trim()
            .parse()
            .map_err(|_| ForgeError::Unreadable(text.into_owned()))
    }

    fn branch_head(&self, branch: &str) -> Result<Option<String>, ForgeError> {
        let path = format!("repos/{}/branches/{branch}", self.repo.as_str());
        match self.hold.call(&["api", &path, "--jq", ".commit.sha"]) {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out);
                match text.trim() {
                    "" => Err(ForgeError::Unreadable(text.into_owned())),
                    sha => Ok(Some(sha.to_owned())),
                }
            }
            Err(error) if error.is_not_found() => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn edit(&self, number: NonZeroU64, title: &str, body: &str) -> Result<(), ForgeError> {
        self.check(title, body)?;
        let args = edit_args(&self.repo, number, title, body);
        self.hold.call(&as_strs(&args)).map(drop)
    }

    fn state(&self, number: NonZeroU64) -> Result<PullState, ForgeError> {
        let number = number.to_string();
        let args = [
            "pr",
            "view",
            &number,
            "--repo",
            self.repo.as_str(),
            "--json",
            "state",
        ];
        let out = self.hold.call(&args)?;
        parse_state(&out)
    }

    fn mark_ready(&self, number: NonZeroU64) -> Result<(), ForgeError> {
        let number = number.to_string();
        let args = ["pr", "ready", &number, "--repo", self.repo.as_str()];
        self.hold.call(&args).map(drop)
    }

    fn merge(&self, number: NonZeroU64, head: &str) -> Result<(), ForgeError> {
        let number = number.to_string();
        self.hold
            .call(&merge_args(&self.repo, &number, head))
            .map(drop)
    }

    fn disable_auto_merge(&self, number: NonZeroU64) -> Result<(), ForgeError> {
        let number = number.to_string();
        let args = [
            "pr",
            "merge",
            &number,
            "--repo",
            self.repo.as_str(),
            "--disable-auto",
        ];
        self.hold.call(&args).map(drop)
    }
}

fn parse_state(stdout: &[u8]) -> Result<PullState, ForgeError> {
    #[derive(Deserialize)]
    struct View {
        state: String,
    }
    let unreadable = || ForgeError::Unreadable(String::from_utf8_lossy(stdout).into_owned());
    let view: View = serde_json::from_slice(stdout).map_err(|_| unreadable())?;
    match view.state.as_str() {
        "OPEN" => Ok(PullState::Open),
        "MERGED" => Ok(PullState::Merged),
        "CLOSED" => Ok(PullState::Closed),
        _ => Err(unreadable()),
    }
}

// A merge commit, and only of the head asked about. No `--delete-branch`: gh
// would also switch branches in its working folder. An empty `--body`: GitHub's
// default is the title, which a release tool reads as one more commit.
fn merge_args<'a>(repo: &'a ForgeSlug, number: &'a str, head: &'a str) -> [&'a str; 10] {
    [
        "pr",
        "merge",
        number,
        "--repo",
        repo.as_str(),
        "--merge",
        "--match-head-commit",
        head,
        "--body",
        "",
    ]
}

fn as_strs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

// A draft, always: it is marked ready when it is merged. `-f` sends text as it
// is, so a text that begins with a dash is not read as a flag.
fn create_args(repo: &ForgeSlug, head: &str, base: &str, title: &str, body: &str) -> Vec<String> {
    let path = format!("repos/{}/pulls", repo.as_str());
    let mut args = ["api", "--method", "POST", &path]
        .map(str::to_owned)
        .to_vec();
    for field in [
        format!("title={title}"),
        format!("body={body}"),
        format!("head={head}"),
        format!("base={base}"),
    ] {
        args.extend(["-f".to_owned(), field]);
    }
    args.extend(["-F", "draft=true", "--jq", ".number"].map(str::to_owned));
    args
}

fn edit_args(repo: &ForgeSlug, number: NonZeroU64, title: &str, body: &str) -> Vec<String> {
    let path = format!("repos/{}/pulls/{number}", repo.as_str());
    let mut args = ["api", "--method", "PATCH", &path]
        .map(str::to_owned)
        .to_vec();
    for field in [format!("title={title}"), format!("body={body}")] {
        args.extend(["-f".to_owned(), field]);
    }
    args
}
