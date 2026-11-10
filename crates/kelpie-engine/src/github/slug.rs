//! A forge repo as `owner/name`
//!
//! Every GitHub call is about one repo, and a token is minted for one repo, so
//! the slug is what a worker's credential is scoped to. [`forge_of`] reads it
//! out of the remote URL a project's checkout carries, which is the same URL a
//! ticket's clone copies ([`crate::worktree::clone`]).

use std::fmt;

/// A forge repo as `owner/name`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeSlug(String);

impl ForgeSlug {
    /// The slug as `owner/name`
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The repo's name, without its owner
    pub fn name(&self) -> &str {
        self.0.split_once('/').map_or(&self.0, |(_, name)| name)
    }

    /// The repo's owner, a user or an organization
    pub fn owner(&self) -> &str {
        self.0.split_once('/').map_or(&self.0, |(owner, _)| owner)
    }
}

impl TryFrom<String> for ForgeSlug {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let part = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        match value.split_once('/') {
            Some((owner, name)) if part(owner) && part(name) => Ok(Self(value)),
            _ => Err("must be `owner/name`"),
        }
    }
}

impl fmt::Display for ForgeSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The GitHub repo a remote's URL names, over HTTPS or SSH
pub fn forge_of(url: &str) -> Option<ForgeSlug> {
    let path = [
        "https://github.com/",
        "http://github.com/",
        "ssh://git@github.com/",
        "git@github.com:",
    ]
    .iter()
    .find_map(|prefix| url.strip_prefix(prefix))?;
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    ForgeSlug::try_from(path.to_owned()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forge_slug_s_name_is_the_repo_without_its_owner() {
        let slug = ForgeSlug::try_from("shep-pm/koji-website".to_owned()).unwrap();
        assert_eq!((slug.name(), slug.owner()), ("koji-website", "shep-pm"));
    }

    #[test]
    fn a_slug_outside_owner_name_is_refused() {
        for bad in ["koji", "", "/koji", "shep-pm/", "a/b/c", "a/b c"] {
            let err = ForgeSlug::try_from(bad.to_owned()).unwrap_err();
            assert_eq!(err, "must be `owner/name`", "{bad:?}");
        }
    }

    #[test]
    fn a_github_remote_names_its_repo_over_https_or_ssh() {
        for url in [
            "https://github.com/shep-pm/koji-website.git",
            "https://github.com/shep-pm/koji-website",
            "git@github.com:shep-pm/koji-website.git",
            "ssh://git@github.com/shep-pm/koji-website/",
        ] {
            let slug = forge_of(url).unwrap_or_else(|| panic!("{url}"));
            assert_eq!(slug.as_str(), "shep-pm/koji-website", "{url}");
        }
    }

    #[test]
    fn a_remote_elsewhere_names_no_repo() {
        for url in [
            "https://gitlab.com/a/b.git",
            "/srv/git/b.git",
            "https://github.com/a",
            "https://github.com/a/b/c",
        ] {
            assert_eq!(forge_of(url), None, "{url}");
        }
    }
}
