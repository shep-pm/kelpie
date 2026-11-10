//! The Harness global folder setting, tried against a real sandbox
//!
//! A session reads the one entry of the person's own global folder its harness
//! linked into it, and nothing else there, at either setting for how far a
//! project reads (ADR 0005, ADR 0006). A child of `tests/sandbox.rs`, which is
//! its own `main` (crabpot's Linux helper is this binary run again).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;

use kelpie_engine::harness::global;
use kelpie_engine::sandbox::{Project, Reads, policy};

use super::{SKIPPED, Scratch, harness_in, sandbox_in, sh, working};

/// The argument that makes this binary run the global folder checks with `HOME` moved
/// into the scratch folder, which is how a test reaches a path the policy denies
pub const IN_HOME: &str = "--in-the-persons-home";

/// What that child exits with when this host runs no sandbox at all
const CHILD_SKIPPED: i32 = 3;

/// A session reads the entry of the person's global folder its harness linked, and nothing
/// else there, whatever the setting it is run with
///
/// Every path the policy denies lies under `~`, so the checks run in a child whose `HOME`
/// is the scratch folder: only there is the person's folder one the policy closes and a
/// link reaches into. `set_var` is unsafe, which the crate forbids, so the scratch is
/// named to the child instead of set here.
pub fn the_global_folder_is_read_only_when_on() {
    let scratch = Scratch::new("global-folder");
    // The person's own Claude Code folder, under the home folder the child is run
    // with, which is where the harness keeps the global config the setting links
    let global = scratch.folder(".claude");
    let (login, linked) = (global.join(".credentials.json"), global.join("CLAUDE.md"));
    fs::write(&login, "their login\n").unwrap();
    fs::write(&linked, "the person's own instructions\n").unwrap();

    let run = Command::new(std::env::current_exe().expect("this binary"))
        .arg(IN_HOME)
        .arg(&scratch.root)
        .env("HOME", &scratch.root)
        .output()
        .expect("the child runs with the scratch folder as its home folder");

    match run.status.code() {
        Some(CHILD_SKIPPED) => {
            SKIPPED.fetch_add(1, Ordering::SeqCst);
            println!(
                "SKIPPED the_global_folder_is_read_only_when_on: the child found no sandbox to run"
            );
        }
        Some(0) => {}
        _ => panic!(
            "the linked entry, the login beside it or a write through the link was not what the policy says: {}",
            String::from_utf8_lossy(&run.stderr)
        ),
    }
}

/// The checks of [`the_global_folder_is_read_only_when_on`], with `root` as this home folder
pub fn in_the_persons_home(root: &Path) -> i32 {
    // The person's own Claude Code folder, under the home folder this child is run
    // with: the policy closes their login by its own name under the home folder,
    // so a global folder anywhere else names nothing the policy denies
    let (global, project, state, sessions_root) = (
        root.join(".claude"),
        root.join("work/project"),
        root.join("state"),
        root.join("data/projects"),
    );
    let (login, linked) = (global.join(".credentials.json"), global.join("CLAUDE.md"));
    // The session's config folder, holding the person's entry as a harness builder links it
    let (config, through) = (state.join("claude"), state.join("claude").join("CLAUDE.md"));
    let entries = global::link(&global, &config, &["CLAUDE.md"]).unwrap_or_else(|error| {
        panic!("the person's entry was not linked into the session: {error}")
    });

    // The policy is what every check below leans on, and reading it needs no
    // sandbox, so it is read first: on a host that cannot start one, this is the
    // part of the test that still says whether the scratch is a world the policy
    // closes
    denies_the_login(&project, &state, &sessions_root, &entries, &login);

    // A session on the linked entry, at each setting for how far it reads
    for reads in [Reads::Project, Reads::Anywhere] {
        let Some(session) = working(
            sandbox_in(
                Path::new("/bin/sh"),
                &project,
                &state,
                &sessions_root,
                reads,
                &entries,
                None,
            ),
            "the_global_folder_is_read_only_when_on",
        ) else {
            return CHILD_SKIPPED;
        };
        let (read, login_read, written) = (
            sh(&session, &format!("cat {}", through.display())),
            sh(&session, &format!("cat {}", login.display())),
            sh(&session, &format!("echo nope > {}", through.display())),
        );
        assert!(
            read.status.success(),
            "the linked entry was not read through the session's name for it, at {reads:?}: {read:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&read.stdout),
            "the person's own instructions\n",
            "the entry read at {reads:?} is not the person's own file"
        );
        assert!(
            !login_read.status.success(),
            "the person's login was read through their global folder, at {reads:?}: {login_read:?}"
        );
        assert!(
            !written.status.success(),
            "a write through the linked entry was not refused, at {reads:?}: {written:?}"
        );
        assert_eq!(
            fs::read_to_string(&linked).unwrap(),
            "the person's own instructions\n",
            "a sandboxed write at {reads:?} changed the person's own file"
        );
        drop(session);
    }

    // The same session with the setting off: the link is taken back, and the person's
    // own file is closed again
    let cleared = global::link(&global, &config, &[])
        .unwrap_or_else(|error| panic!("the link was not taken back: {error}"));
    assert!(cleared.is_empty(), "{cleared:?}");
    assert!(!through.exists(), "the link stayed in the session's folder");

    let Some(off) = working(
        sandbox_in(
            Path::new("/bin/sh"),
            &project,
            &state,
            &sessions_root,
            Reads::Project,
            &[],
            None,
        ),
        "the_global_folder_is_read_only_when_on",
    ) else {
        return CHILD_SKIPPED;
    };
    let read = sh(&off, &format!("cat {}", linked.display()));
    assert!(
        !read.status.success(),
        "the person's own file was read with nothing linked into the session: {read:?}"
    );
    0
}

/// Refuse the scratch when its global folder is nowhere the policy closes a login
///
/// Each check of [`in_the_persons_home`] leans on the policy, and reading it needs
/// no sandbox, so it is read first: on a host that cannot start one, this is the
/// part of the test that still says whether the person's folder is one the policy
/// names. With *Read anywhere by default* on the home folder is open and only a
/// login closed by its own name stays shut, which is the setting that shows a
/// global folder put anywhere else names nothing the policy denies at all.
fn denies_the_login(
    project: &Path,
    state: &Path,
    sessions_root: &Path,
    entries: &[PathBuf],
    login: &Path,
) {
    let harness = harness_in(Path::new("/bin/sh"), entries);
    let allowlist: Vec<String> = Vec::new();
    let confined = Project {
        folder: project,
        network_allowlist: &allowlist,
        reads: Reads::Anywhere,
        sessions_root,
    };
    let settings = policy(&harness, &confined, state)
        .unwrap_or_else(|error| panic!("the policy could not be built: {error}"));
    let denied = settings
        .filesystem
        .read_denies()
        .unwrap_or_else(|error| panic!("the policy's read denies could not be read: {error}"));
    let wanted = login.to_string_lossy().into_owned();
    assert!(
        denied.iter().any(|rule| rule.path() == wanted),
        "the policy closes no login named {wanted}, so the scratch's global folder \
         is not the one the harness keeps under its home folder: {denied:?}"
    );
}
