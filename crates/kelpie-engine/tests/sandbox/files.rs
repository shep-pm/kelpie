//! What a sandboxed command reads and writes: the project, the state folder, and
//! nothing beside them
//!
//! A child of `tests/sandbox.rs`, which is its own `main` (crabpot's Linux helper
//! is this binary run again).

use std::fs;
use std::path::{Path, PathBuf};

use kelpie_engine::sandbox::{Project, Reads, Sandbox};
use kelpie_engine::session::Harness;

use super::escapes;
use super::host::{host_can_run, working_sandbox};
use super::support::{HomeFolder, Scratch, sh};

pub(super) fn a_write_outside_the_state_dir_is_refused() {
    let scratch = Scratch::new("refused");
    let Some(sandbox) = working_sandbox(
        &scratch,
        "a_write_outside_the_state_dir_is_refused",
        Path::new("/bin/sh"),
        Reads::Project,
    ) else {
        return;
    };
    // The control: a write the policy allows lands, so the refusals below are
    // the policy's and not a sandbox that ran nothing
    let allowed = scratch.state().join("control");
    let control = sh(&sandbox, &format!("echo yes > {}", allowed.display()));
    assert!(control.status.success(), "{control:?}");
    assert!(allowed.exists(), "a write to the state folder did not land");
    let outside = scratch.outside().join("nope");
    let project = scratch.project().join("nope");

    let to_outside = sh(&sandbox, &format!("echo no > {}", outside.display()));
    let to_project = sh(&sandbox, &format!("echo no > {}", project.display()));

    for (name, run, path) in [
        ("outside", to_outside, outside),
        ("project", to_project, project),
    ] {
        assert!(
            !run.status.success(),
            "a write to the {name} folder was not refused: {run:?}"
        );
        assert!(!path.exists(), "a write to the {name} folder landed");
    }
}

pub(super) fn a_sandboxed_command_reads_the_project_and_writes_the_state_dir() {
    let scratch = Scratch::new("allowed");
    fs::write(scratch.project().join("notes.md"), "from the project").unwrap();
    let Some(sandbox) = working_sandbox(
        &scratch,
        "a_sandboxed_command_reads_the_project_and_writes_the_state_dir",
        Path::new("/bin/sh"),
        Reads::Project,
    ) else {
        return;
    };
    let written = scratch.state().join("claude").join("out");
    fs::create_dir_all(written.parent().unwrap()).unwrap();

    let run = sh(
        &sandbox,
        &format!(
            "cat {} > {}",
            scratch.project().join("notes.md").display(),
            written.display()
        ),
    );

    assert!(run.status.success(), "{run:?}");
    assert_eq!(fs::read_to_string(&written).unwrap(), "from the project");
}

/// `path` read by `sh` inside `sandbox`, `None` when the sandbox refused it
fn cat(sandbox: &Sandbox, path: &Path) -> Option<String> {
    let run = sh(sandbox, &format!("cat {}", path.display()));
    run.status
        .success()
        .then(|| String::from_utf8_lossy(&run.stdout).into_owned())
}

/// A real read outside the project follows the setting
///
/// The area `Reads::Project` closes is the home folder and the folder holding
/// every session's state. crabpot allows every read the policy does not deny
/// (ADR 0005's reads wide), so a file in some other folder is read at either
/// setting and proves nothing: the file outside the project has to be one the
/// policy closes.
pub(super) fn a_real_read_outside_the_project_follows_the_setting() {
    let name = "a_real_read_outside_the_project_follows_the_setting";
    let (scratch, home) = (
        Scratch::new("read-anywhere"),
        HomeFolder::new("read-anywhere"),
    );
    let (project, outside) = (scratch.project().join("notes.md"), home.file("notes.md"));
    let (other, own) = (scratch.other().join("log"), scratch.state().join("log"));
    for (file, text) in [
        (&project, "in the project"),
        (&outside, "outside the project"),
        (&other, "another session's state"),
        (&own, "this session's own state"),
    ] {
        fs::write(file, text).expect("the scratch files are written");
    }

    let Some(closed) = working_sandbox(&scratch, name, Path::new("/bin/sh"), Reads::Project) else {
        return;
    };
    assert_eq!(cat(&closed, &project).as_deref(), Some("in the project"));
    assert_eq!(
        cat(&closed, &own).as_deref(),
        Some("this session's own state"),
        "the session cannot read its own state folder with Read anywhere off"
    );
    let refused = cat(&closed, &outside);
    assert!(
        refused.is_none(),
        "a file in the home folder was read with Read anywhere off: {refused:?}"
    );
    drop(closed);

    let Some(wide) = working_sandbox(&scratch, name, Path::new("/bin/sh"), Reads::Anywhere) else {
        return;
    };
    assert_eq!(
        cat(&wide, &outside).as_deref(),
        Some("outside the project"),
        "Read anywhere did not reach outside the project"
    );
    assert_eq!(
        cat(&wide, &own).as_deref(),
        Some("this session's own state"),
        "the session cannot read its own state folder"
    );
    let other_read = cat(&wide, &other);
    assert!(
        other_read.is_none(),
        "another session's state was read: {other_read:?}"
    );
}

/// A denied folder under the allowed folder beside a project stays closed
///
/// The rule `reads` and `in_denied` are built on, run for real: crabpot masks a
/// denied path again when an allowed folder sits above it, so a project at
/// `<home>/.config/nvim` reads `<home>/.config/other` beside it and reads nothing
/// of the `<home>/.config/gh` beside that. An allow at or inside a denied folder
/// is the other half, and `sandbox::tests` holds it.
///
/// In a home of its own, so the stand-in for a login never lands in the person's
/// own `~/.config`.
pub(super) fn a_denied_folder_under_an_allowed_one_stays_closed() {
    escapes::in_own_home("mask");
}

/// The case `in_own_home` runs for `mask`
pub(super) fn a_denied_folder_under_an_allowed_one() {
    let home = fs::canonicalize(std::env::var_os("HOME").expect("the case has a home folder"))
        .expect("the home folder is there");
    let (folder, beside, secret) = (
        home.join(".config/nvim"),
        home.join(".config/other"),
        home.join(".config/gh"),
    );
    let (sessions_root, state) = (home.join("projects"), home.join("projects/pm"));
    for held in [&folder, &beside, &secret, &state] {
        fs::create_dir_all(held).expect("the case's folders are made");
    }
    for (file, text) in [
        (folder.join("notes.md"), "the project's own\n"),
        (beside.join("file"), "beside the project\n"),
        (secret.join("hosts.yml"), "a login\n"),
    ] {
        fs::write(&file, text).expect("the case's file is written");
    }

    let mut harness = Harness::new("/bin/sh");
    harness.installs = vec![PathBuf::from("/bin")];
    let allowlist: Vec<String> = Vec::new();
    let project = Project {
        folder: &folder,
        network_allowlist: &allowlist,
        reads: Reads::Project,
        sessions_root: &sessions_root,
    };
    let sandbox = Sandbox::start(&harness, &project, &state)
        .expect("a project beside a denied folder starts a sandbox");
    let name = "a_denied_folder_under_an_allowed_one_stays_closed";
    if !host_can_run(name, &sh(&sandbox, "true")) {
        return;
    }

    // The control: the folder beside the project is read, so the refusal below is
    // the policy's and not a sandbox that reads nothing
    assert_eq!(
        cat(&sandbox, &beside.join("file")).as_deref(),
        Some("beside the project\n"),
        "the folder beside the project was not read"
    );
    let read = cat(&sandbox, &secret.join("hosts.yml"));
    assert!(
        read.is_none(),
        "the denied folder under the allowed one was read: {read:?}"
    );
}
