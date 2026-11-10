//! The Harness global folder setting, tried against a real sandbox
//!
//! A session reads the one entry of the person's own global folder its harness
//! linked into it, and nothing else there (ADR 0006). A child of
//! `tests/sandbox.rs`, which is its own `main` (crabpot's Linux helper is this
//! binary run again).

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::Ordering;

use kelpie_engine::harness::global;
use kelpie_engine::sandbox::Reads;

use super::{SKIPPED, Scratch, sandbox_in, sh, working};

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
    // The person's own folder, kept out of the folders beside the project so only
    // what is linked can be read
    let global = scratch.folder("home/.claude");
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
    let (global, project, state, sessions_root) = (
        root.join("home/.claude"),
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

    let Some(on) = working(
        sandbox_in(
            Path::new("/bin/sh"),
            &project,
            &state,
            &sessions_root,
            Reads::Project,
            &entries,
            None,
        ),
        "the_global_folder_is_read_only_when_on",
    ) else {
        return CHILD_SKIPPED;
    };

    let read = sh(&on, &format!("cat {}", through.display()));
    assert!(
        read.status.success(),
        "the linked entry was not read through the session's name for it: {read:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&read.stdout),
        "the person's own instructions\n"
    );

    let login_read = sh(&on, &format!("cat {}", login.display()));
    assert!(
        !login_read.status.success(),
        "the person's login was read through their global folder: {login_read:?}"
    );

    let written = sh(&on, &format!("echo nope > {}", through.display()));
    assert!(
        !written.status.success(),
        "a write through the linked entry was not refused: {written:?}"
    );
    assert_eq!(
        fs::read_to_string(&linked).unwrap(),
        "the person's own instructions\n",
        "a sandboxed write changed the person's own file"
    );
    drop(on);

    // The same session with *Read anywhere by default* on: the home folder is open,
    // and the login beside the linked entry is still closed by its own name
    let Some(wide) = working(
        sandbox_in(
            Path::new("/bin/sh"),
            &project,
            &state,
            &sessions_root,
            Reads::Anywhere,
            &entries,
            None,
        ),
        "the_global_folder_is_read_only_when_on",
    ) else {
        return CHILD_SKIPPED;
    };

    let read = sh(&wide, &format!("cat {}", through.display()));
    assert!(
        read.status.success(),
        "the linked entry was not read with Read anywhere on: {read:?}"
    );
    let login_read = sh(&wide, &format!("cat {}", login.display()));
    assert!(
        !login_read.status.success(),
        "the person's login was read with Read anywhere on: {login_read:?}"
    );
    let written = sh(&wide, &format!("echo nope > {}", through.display()));
    assert!(
        !written.status.success(),
        "a write through the linked entry was not refused with Read anywhere on: {written:?}"
    );
    assert_eq!(
        fs::read_to_string(&linked).unwrap(),
        "the person's own instructions\n",
        "a sandboxed write with Read anywhere on changed the person's own file"
    );
    drop(wide);

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
