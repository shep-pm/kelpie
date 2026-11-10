use std::path::Path;

#[cfg(unix)]
use super::secrets::Linked;
use super::*;

#[test]
fn read_anywhere_writes_only_the_state_dir() {
    let harness = installed(on(&["api.anthropic.com"]), &["/opt/claude"]);

    for reads in [Reads::Project, Reads::Anywhere] {
        let config = built(&harness, &["docs.rs".to_owned()], reads);

        assert_eq!(
            config.filesystem.allow_write,
            entries(&[STATE]).filesystem.allow_write,
            "with {reads:?}"
        );
        for log in ["~/.npm/_logs", "~/.claude/debug"] {
            assert!(
                writes_denied(&config, log),
                "{log:?} is writable with {reads:?}"
            );
        }
    }
}

#[test]
fn reads_stay_in_the_project_by_default() {
    let harness = installed(on(&[]), &[INSTALL, PROJECT]);

    let config = built(&harness, &[], Reads::Project);

    assert_eq!(
        config.filesystem.allow_read,
        entries(&[PROJECT, BESIDE, STATE, INSTALL])
            .filesystem
            .allow_read
    );
    assert!(denies(&config, "~"), "the home folder is closed");
}

#[test]
fn read_anywhere_reads_outside_the_project() {
    let harness = installed(on(&[]), &[INSTALL]);

    let config = built(&harness, &[], Reads::Anywhere);

    assert!(!denies(&config, "~"), "the home folder is closed");
    // Only what the policy closes itself is opened back. The folders beside the
    // project are named nowhere, since nothing closes them.
    assert_eq!(
        config.filesystem.allow_read,
        entries(&[PROJECT, STATE, INSTALL]).filesystem.allow_read
    );
}

/// The only folders a policy close reads are the home folder and the folder
/// holding every session's state
///
/// crabpot allows every read the policy does not deny, so every other folder on
/// the machine is read at either setting. A test that wants a real read to
/// follow the setting has to put its file inside one of these two.
#[test]
fn nothing_but_the_home_folder_and_the_sessions_root_is_closed_to_reads() {
    for reads in [Reads::Project, Reads::Anywhere] {
        let config = built(&on(&[]), &[], reads);

        let mut closed: Vec<String> = denied().into_iter().map(str::to_owned).collect();
        closed.push(format!("{SESSIONS}/**"));
        if reads == Reads::Project {
            closed.push("~".to_owned());
        }
        let closed = closed.iter().map(String::as_str).collect::<Vec<_>>();

        assert_eq!(
            config.filesystem.deny_read,
            entries(&closed).filesystem.deny_read,
            "the closed area is more than the home folder and the sessions root with {reads:?}"
        );
    }
}

#[test]
fn every_other_sessions_state_stays_out_of_reach() {
    for reads in [Reads::Project, Reads::Anywhere] {
        let config = built(&on(&[]), &[], reads);

        assert!(denies(&config, &format!("{SESSIONS}/**")), "{reads:?}");
        assert!(
            allows(&config, STATE),
            "the session cannot read its own state folder with {reads:?}"
        );
    }
}

/// The guard, which is the second line: no allow lands on the home folder
///
/// `Sandbox::start` refuses a project there outright, so what the guard has left
/// to catch is every other folder an allow could name.
#[test]
fn no_allow_lands_on_the_home_folder() {
    let home = home();
    let text = home.to_string_lossy().into_owned();
    let harness = installed(on(&[]), &[text.as_str()]);

    let inside = home.join("kelpie");

    for reads in [Reads::Project, Reads::Anywhere] {
        // The project at the home folder, beside it at the home folder, and a
        // harness installed there
        let at_home = Project {
            folder: &home,
            network_allowlist: &[],
            reads,
            sessions_root: Path::new(SESSIONS),
        };
        let beside_home = Project {
            folder: &inside,
            network_allowlist: &[],
            reads,
            sessions_root: Path::new(SESSIONS),
        };
        let installed_at_home = Project {
            folder: Path::new(PROJECT),
            network_allowlist: &[],
            reads,
            sessions_root: Path::new(SESSIONS),
        };

        for project in [&at_home, &beside_home, &installed_at_home] {
            let config = policy(&harness, project, Path::new(STATE)).unwrap();

            assert!(
                !allows(&config, &text),
                "the home folder is opened back with {reads:?}"
            );
            assert!(denies(&config, "~/.ssh/**"), "{reads:?}");
        }
    }
}

/// The guard, on a project whose own folder is inside a denied one
///
/// `Sandbox::start` refuses such a project outright, so this builds the reads
/// directly: the project, the folder beside it and a harness installed in the
/// denied folder are each named, and none of them is.
#[test]
fn no_allow_lands_inside_a_denied_folder() {
    let denied = home().join(".pi");
    let folder = denied.join("agent/x");
    let harness = installed(on(&[]), &[denied.join("agent")]);
    let allowlist: Vec<String> = Vec::new();

    for reads in [Reads::Project, Reads::Anywhere] {
        let project = Project {
            folder: &folder,
            network_allowlist: &allowlist,
            reads,
            sessions_root: Path::new(SESSIONS),
        };
        let allowed = policy::reads(&harness, &project, Path::new(STATE));
        let held = resolved(&denied);

        assert!(
            !allowed.contains(&denied.join("agent").to_string_lossy().into_owned()),
            "the folder beside the project opens {denied:?} back with {reads:?}: {allowed:?}"
        );
        for entry in &allowed {
            assert!(
                !resolved(Path::new(entry)).starts_with(&held),
                "{entry} is at or inside the denied folder {denied:?} with {reads:?}"
            );
        }
        assert!(
            allowed.contains(&STATE.to_owned()),
            "the session cannot read its own state folder with {reads:?}: {allowed:?}"
        );
    }
}

/// A folder holding a denied folder, and not itself one, is read as it was
///
/// crabpot lands the deny of `~/.config/gh` after the allow of the folder above
/// it, so a project there needs no refusal. `tests/sandbox.rs` runs the read
/// itself.
#[test]
fn a_project_above_a_denied_folder_still_starts() {
    let folder = home().join(".config/nvim");
    let project = Project {
        folder: &folder,
        network_allowlist: &[],
        reads: Reads::Project,
        sessions_root: Path::new(SESSIONS),
    };

    check_project(&project)
        .unwrap_or_else(|error| panic!("a project beside a denied folder is refused: {error}"));

    let allowed = policy::reads(&on(&[]), &project, Path::new(STATE));
    let beside = folder
        .parent()
        .expect("the OS puts the project in a folder");
    for wanted in [&folder, beside] {
        assert!(
            allowed.contains(&wanted.to_string_lossy().into_owned()),
            "{wanted:?} is not read: {allowed:?}"
        );
    }
}

/// The second line, on every folder an allow could name through a link
///
/// A policy built without `Sandbox::start` still keeps the home folder and the
/// sessions root closed, whichever way the project reads.
#[cfg(unix)]
#[test]
fn no_allow_lands_on_a_closed_folder_through_a_link() {
    let linked = Linked::new("closed");
    let (folder, sessions_root) = (linked.project(), linked.sessions_root());
    let state = sessions_root.join("pm");
    // The home folder, the folder holding it, and the sessions root the link
    // above names, each spelled the way the OS does not
    let installs = [linked.home(), linked.above(), linked.real_sessions_root()];
    let harness = installed(on(&[]), &installs);
    let allowlist: Vec<String> = Vec::new();

    for reads in [Reads::Project, Reads::Anywhere] {
        let project = Project {
            folder: &folder,
            network_allowlist: &allowlist,
            reads,
            sessions_root: &sessions_root,
        };
        let config = policy(&harness, &project, &state).unwrap();

        for named in &installs {
            assert!(
                !allows(&config, &named.to_string_lossy()),
                "{named:?} is read back with {reads:?}"
            );
        }
        assert!(
            allows(&config, &folder.to_string_lossy()),
            "the project is not read with {reads:?}"
        );
        assert!(
            allows(&config, &state.to_string_lossy()),
            "the session's own state is not read with {reads:?}"
        );
        assert!(denies(&config, "~/.ssh/**"), "{reads:?}");
    }
}

/// The second line, on the project itself
///
/// A policy built without `Sandbox::start` leaves the project's own allow out
/// when the project links to the home folder, so the home folder stays closed.
#[cfg(unix)]
#[test]
fn a_project_that_links_to_the_home_folder_is_not_read_back() {
    let linked = Linked::new("project-at-home");
    let (folder, sessions_root) = (linked.home(), linked.sessions_root());
    let state = sessions_root.join("pm");
    let allowlist: Vec<String> = Vec::new();

    for reads in [Reads::Project, Reads::Anywhere] {
        let project = Project {
            folder: &folder,
            network_allowlist: &allowlist,
            reads,
            sessions_root: &sessions_root,
        };
        let config = policy(&on(&[]), &project, &state).unwrap();

        assert!(
            !allows(&config, &folder.to_string_lossy()),
            "the project opens the home folder back with {reads:?}"
        );
        assert!(
            !allows(&config, &home().to_string_lossy()),
            "the home folder is opened back with {reads:?}"
        );
        assert!(denies(&config, "~/.ssh/**"), "{reads:?}");
    }
}

#[test]
fn the_folders_beside_a_project_are_left_out_when_they_would_open_the_root() {
    let folder = Path::new("/kelpie");
    let project = Project {
        folder,
        network_allowlist: &[],
        reads: Reads::Project,
        sessions_root: Path::new(SESSIONS),
    };

    let config = policy(&on(&[]), &project, Path::new(STATE)).unwrap();

    assert_eq!(
        config.filesystem.allow_read,
        entries(&["/kelpie", STATE]).filesystem.allow_read
    );
}
