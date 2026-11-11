//! The draft pull request, its edit and a branch's head, through a scripted
//! [`Runner`], so no test needs `gh`, a login or the network
//!
//! [`Runner`]: super::Runner

use std::num::NonZeroU64;
use std::path::Path;

use super::hold::Hold;
use super::pulls::GithubPulls;
use super::tests::{Script, answered, gh_over, refused};
use crate::github::ForgeSlug;
use crate::local_paths::LocalPaths;
use crate::ticket::{ForgeError, Pulls};

const FOLDER: &str = "/work/koji";

fn pulls_over(script: &Script) -> GithubPulls {
    let repo = ForgeSlug::try_from("shep-pm/koji".to_owned()).unwrap();
    let paths = LocalPaths::new([Path::new(FOLDER)]);
    GithubPulls::new(Hold::new(gh_over(script)), repo, paths)
}

fn number(n: u64) -> NonZeroU64 {
    NonZeroU64::new(n).unwrap()
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

// `-f` sends the text as it is, where a flag for the title or body could read
// text that begins with a dash as a flag; `-F draft=true` is the one typed field.
#[test]
fn a_pull_request_is_opened_as_a_draft_with_its_text_sent_as_fields() {
    let script = Script::answering(answered("12\n"));

    pulls_over(&script)
        .create_draft("kelpie/7", "main", "feat: x", "- body\n\nResolves #7")
        .unwrap();

    assert_eq!(
        script.asked(),
        [strings(&[
            "api",
            "--method",
            "POST",
            "repos/shep-pm/koji/pulls",
            "-f",
            "title=feat: x",
            "-f",
            "body=- body\n\nResolves #7",
            "-f",
            "head=kelpie/7",
            "-f",
            "base=main",
            "-F",
            "draft=true",
            "--jq",
            ".number",
        ])]
    );
}

#[test]
fn an_edit_changes_the_title_and_body_of_the_pull_request_alone() {
    let script = Script::answering(answered(""));

    pulls_over(&script)
        .edit(number(12), "fix: y", "body")
        .unwrap();

    assert_eq!(
        script.asked(),
        [strings(&[
            "api",
            "--method",
            "PATCH",
            "repos/shep-pm/koji/pulls/12",
            "-f",
            "title=fix: y",
            "-f",
            "body=body",
        ])]
    );
}

#[test]
fn the_number_gh_prints_is_the_pull_requests_number() {
    let script = Script::answering(answered("  345\n"));

    let opened = pulls_over(&script).create_draft("kelpie/7", "main", "t", "b");

    assert_eq!(opened, Ok(number(345)));
}

#[test]
fn output_that_is_not_a_number_is_unreadable() {
    for printed in ["not a number\n", "", "0\n"] {
        let script = Script::answering(answered(printed));

        let opened = pulls_over(&script).create_draft("kelpie/7", "main", "t", "b");

        assert_eq!(opened, Err(ForgeError::Unreadable(printed.to_owned())));
    }
}

#[test]
fn a_title_or_body_naming_the_projects_folder_is_refused_before_any_child_starts() {
    let script = Script::default();
    let pulls = pulls_over(&script);

    let titled = pulls.create_draft("kelpie/7", "main", "fix /work/koji/src", "b");
    let bodied = pulls.create_draft("kelpie/7", "main", "t", "see /work/koji/src/lib.rs");
    let edited = pulls.edit(number(3), "t", "in %2Fwork%2Fkoji");

    assert_eq!(titled, Err(ForgeError::LocalPath));
    assert_eq!(bodied, Err(ForgeError::LocalPath));
    assert_eq!(edited, Err(ForgeError::LocalPath));
    assert_eq!(script.asked(), Vec::<Vec<String>>::new());
}

#[test]
fn a_home_folder_path_in_the_body_is_refused_and_the_error_does_not_echo_it() {
    let script = Script::default();

    let refused = pulls_over(&script)
        .create_draft("kelpie/7", "main", "t", "built in /home/alice/code")
        .unwrap_err();

    assert_eq!(refused, ForgeError::LocalPath);
    assert!(!format!("{refused} {refused:?}").contains("alice"));
    assert_eq!(script.asked(), Vec::<Vec<String>>::new());
}

#[test]
fn branch_head_is_the_sha_gh_prints_and_none_on_a_404() {
    let script = Script::answering(answered("0123abc\n"));
    script.then(refused("gh: Branch not found (HTTP 404)"));
    script.then(refused("gh: Server Error (HTTP 500)"));
    let pulls = pulls_over(&script);

    let pushed = pulls.branch_head("kelpie/7");
    let missing = pulls.branch_head("kelpie/8");
    let broken = pulls.branch_head("kelpie/9");

    assert_eq!(pushed, Ok(Some("0123abc".to_owned())));
    assert_eq!(missing, Ok(None));
    assert_eq!(
        broken,
        Err(ForgeError::Failed("gh: Server Error (HTTP 500)".to_owned()))
    );
    assert_eq!(
        script.asked()[0],
        strings(&[
            "api",
            "repos/shep-pm/koji/branches/kelpie/7",
            "--jq",
            ".commit.sha"
        ])
    );
}

#[test]
fn a_held_limit_stops_the_call() {
    let script = Script::answering(refused(
        "< HTTP/2.0 403 Forbidden\n< Retry-After: 90\n<\n\
         gh: You have exceeded a secondary rate limit. (HTTP 403)\n",
    ));
    let pulls = pulls_over(&script);
    pulls.branch_head("kelpie/7").unwrap_err();

    let opened = pulls.create_draft("kelpie/7", "main", "t", "b");
    let edited = pulls.edit(number(3), "t", "b");
    let head = pulls.branch_head("kelpie/7");

    for answer in [opened.map(drop), edited, head.map(drop)] {
        assert!(matches!(answer, Err(ForgeError::Held(_))), "{answer:?}");
    }
    assert_eq!(script.asked().len(), 1, "a held call starts no child");
}
