//! The tickets of a GitHub project, read as its issues

use serde::Deserialize;

use super::conditional::{Etags, PER_PAGE};
use super::hold::Hold;
use super::listing::{self, Entry};
use super::unreadable;
use crate::github::{ForgeSlug, Timestamp};
use crate::ticket::{ForgeError, GithubTickets, Issue, Listed, TicketId, Tickets};

/// One GitHub repo's issues, as the ticket port reads them
#[derive(Debug)]
pub struct GithubSource {
    hold: Hold,
    repo: ForgeSlug,
    etags: Etags,
}

impl GithubSource {
    /// The issues of `repo`, asked through `hold`
    #[must_use]
    pub fn new(hold: Hold, repo: ForgeSlug) -> Self {
        Self {
            hold,
            repo,
            etags: Etags::new(),
        }
    }
}

// The fields `gh issue view` is asked for. `url` tells an issue from a pull
// request, which `gh issue view` answers for a pull request's number too.
const VIEW_FIELDS: &str = "title,body,labels,state,url";

// `gh` lists newest first and stops at its limit, so a short limit would hide
// the oldest ticket
const LIST_LIMIT: &str = "1000";

const LIST_FIELDS: &str = "number,title,labels,author,createdAt";

// What `gh` says of a number no issue or pull request holds
const NOTHING: &str = "could not resolve to an issue or pull request";

// A failure that says the repo has nothing under the number is the ticket not
// being one of this source's
fn no_issue_if_nothing(error: ForgeError, ticket: TicketId) -> ForgeError {
    match &error {
        ForgeError::Failed(said) if said.to_ascii_lowercase().contains(NOTHING) => {
            ForgeError::NoIssue { ticket }
        }
        _ => error,
    }
}

fn parse_view(stdout: &[u8], ticket: TicketId) -> Result<Issue, ForgeError> {
    #[derive(Deserialize)]
    struct View {
        title: String,
        body: Option<String>,
        labels: Vec<Label>,
        state: String,
        url: String,
    }
    #[derive(Deserialize)]
    struct Label {
        name: String,
    }
    let view: View = serde_json::from_slice(stdout).map_err(|_| unreadable(stdout))?;
    if view.url.contains("/pull/") || view.state == "MERGED" {
        return Err(ForgeError::NoIssue { ticket });
    }
    Ok(Issue {
        title: view.title,
        body: view.body.unwrap_or_default(),
        labels: view.labels.into_iter().map(|l| l.name).collect(),
        open: view.state != "CLOSED",
    })
}

impl GithubSource {
    // Every issue of the REST list `query` asks for, a page at a time, each a
    // conditional read. GitHub lists pull requests as issues, so those are left out.
    fn entries(&self, query: &str) -> Result<Vec<Entry>, ForgeError> {
        let path = format!("repos/{}/issues?{query}&per_page={PER_PAGE}", self.repo);
        let mut entries = Vec::new();
        for page in self.etags.pages(&self.hold, &path)? {
            entries.extend(listing::rest_page(&page)?);
        }
        Ok(entries)
    }
}

impl Tickets for GithubSource {
    fn issue(&self, id: &TicketId) -> Result<Issue, ForgeError> {
        let number = id.github().ok_or(ForgeError::NoIssue { ticket: *id })?;
        let stdout = self
            .hold
            .call(&[
                "issue",
                "view",
                &number.to_string(),
                "--repo",
                self.repo.as_str(),
                "--json",
                VIEW_FIELDS,
            ])
            .map_err(|e| no_issue_if_nothing(e, *id))?;
        parse_view(&stdout, *id)
    }

    fn ready_issues(&self, label: &str) -> Result<Vec<Listed>, ForgeError> {
        let stdout = self.hold.call(&[
            "issue",
            "list",
            "--repo",
            self.repo.as_str(),
            "--label",
            label,
            "--state",
            "open",
            "--limit",
            LIST_LIMIT,
            "--json",
            LIST_FIELDS,
        ])?;
        listing::gh_list(&stdout)
    }
}

impl GithubTickets for GithubSource {
    fn open_issues(&self) -> Result<Vec<Listed>, ForgeError> {
        let entries = self.entries("state=open")?;
        Ok(entries.into_iter().map(|entry| entry.listed).collect())
    }

    fn closed_issues(&self, since: Timestamp) -> Result<Vec<Listed>, ForgeError> {
        let moment = listing::moment(since).ok_or_else(|| {
            ForgeError::Unreadable(format!("{} seconds is not a moment", since.secs()))
        })?;
        let entries = self.entries(&format!("state=closed&since={moment}&sort=updated"))?;
        Ok(entries
            .into_iter()
            .filter(|entry| entry.closed.is_some_and(|closed| closed >= since))
            .map(|entry| entry.listed)
            .collect())
    }
}
