//! The project's base branch, which the guard keeps the worker from pushing to

use super::*;

#[test]
fn a_push_to_the_projects_base_branch_is_refused_whichever_it_is() {
    let tree = WorkerTree::new();
    tree.git(&["branch", "kelpie/51"]);
    tree.git(&["update-ref", "refs/remotes/origin/v2", "HEAD"]);
    for command in [
        "git push origin v2",
        "git push origin HEAD:v2",
        "git push origin HEAD:refs/heads/v2",
        "git push origin heads/v2",
    ] {
        let why = refusal(tree.bash_on("v2", &tree.path(), command));
        assert!(why.contains("changes `v2`"), "{command}: {why}");
    }
    // `main` is a branch like another once the project's base is `v2`.
    assert_eq!(
        tree.bash_on("v2", &tree.path(), "git push origin HEAD:main"),
        Verdict::Allow
    );
    assert_eq!(
        tree.bash_on("v2", &tree.path(), "git push origin kelpie/51"),
        Verdict::Allow
    );
    // The default base stays `main`.
    let why = refusal(tree.bash("git push origin main"));
    assert!(why.contains("changes `main`"), "{why}");
}

#[test]
fn a_fetch_answered_names_the_projects_base_branch() {
    let tree = WorkerTree::new();
    let why = refusal(tree.bash_on("v2", &tree.path(), "git fetch origin"));
    assert!(why.contains("git merge origin/v2"), "{why}");
}
