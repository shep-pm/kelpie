//! The Harness global folder setting, as the PM's sandbox policy reads it

use std::path::{Path, PathBuf};

use super::*;

/// The person's home folder, which the policy closes over, and three entries of it
/// a harness links into its session's config folder
const THEIR_HOME: &str = "/home/person";
const THEIR_CLAUDE_MD: &str = "/home/person/.claude/CLAUDE.md";
const THEIR_SKILLS: &str = "/home/person/.claude/skills";
const THEIR_GOOSE_CONFIG: &str = "/home/person/.config/goose/config.yaml";

/// A harness with `entries` of the person's own global folder linked into its session
fn linked(mut harness: Harness, entries: &[impl AsRef<Path>]) -> Harness {
    harness.global_reads = entries
        .iter()
        .map(|entry| entry.as_ref().to_path_buf())
        .collect();
    harness
}

#[test]
fn a_linked_entry_of_the_persons_global_folder_joins_the_reads() {
    let harness = linked(
        installed(on(&[]), &[INSTALL]),
        &[THEIR_CLAUDE_MD, THEIR_SKILLS],
    );

    let config = built(&harness, &[], Reads::Project);

    assert_eq!(
        config.filesystem.allow_read,
        entries(&[
            PROJECT,
            BESIDE,
            STATE,
            INSTALL,
            THEIR_CLAUDE_MD,
            THEIR_SKILLS
        ])
        .filesystem
        .allow_read
    );
    for whole in [THEIR_HOME, "/home/person/.claude"] {
        assert!(
            !allows(&config, whole),
            "the person's global folder was opened whole, and not just what was linked into the session: {whole:?}"
        );
    }
}

#[test]
fn a_global_folder_read_opens_no_login_or_secret_and_no_write() {
    // goose keeps its config beside its keys, so its whole config folder is a denied
    // login folder. The linked entry is allowed inside it, which is what crabpot
    // carves the deny for (crabpot-macos `allow_read::late_denies`).
    let harness = linked(on(&[]), &[THEIR_CLAUDE_MD, THEIR_GOOSE_CONFIG]);

    for reads in [Reads::Project, Reads::Anywhere] {
        let config = built(&harness, &[], reads);

        for closed in [
            "~/.claude/.credentials.json",
            "~/.claude.json",
            "~/.config/goose/**",
            "~/.ssh/**",
        ] {
            assert!(
                denies(&config, closed),
                "{closed:?} is readable with {reads:?}"
            );
        }
        assert!(
            allows(&config, THEIR_GOOSE_CONFIG),
            "goose's own config was not opened inside its denied config folder"
        );
        assert_eq!(
            config.filesystem.allow_write,
            entries(&[STATE]).filesystem.allow_write,
            "a harness could write outside the state folder"
        );
        for entry in [THEIR_CLAUDE_MD, THEIR_GOOSE_CONFIG] {
            assert!(
                writes_denied(&config, entry),
                "a harness could write through the link at {entry:?} with {reads:?}"
            );
        }
    }
}

/// goose's own config folder under the person's real home folder, where the policy's
/// login denial really applies
fn goose_folder() -> PathBuf {
    home().join(".config/goose")
}

#[test]
fn gooses_config_is_read_and_its_secrets_beside_it_stay_denied() {
    let config_yaml = goose_folder().join("config.yaml");
    let secrets_yaml = goose_folder().join("secrets.yaml");
    let harness = linked(on(&[]), &[&config_yaml]);

    for reads in [Reads::Project, Reads::Anywhere] {
        let config = built(&harness, &[], reads);

        assert!(
            allows(&config, &text(&config_yaml)),
            "goose's config.yaml is not read with {reads:?}"
        );
        assert!(
            !allows(&config, &text(&secrets_yaml)),
            "goose's secrets.yaml was opened with {reads:?}"
        );
        assert!(
            !allows(&config, &text(&goose_folder())),
            "goose's config folder was opened whole with {reads:?}"
        );
        assert!(
            denies(&config, "~/.config/goose/**"),
            "goose's config folder is no longer denied with {reads:?}"
        );
    }
}

#[test]
fn a_global_read_that_is_itself_a_login_or_a_secret_is_refused() {
    let claude_json = home().join(".claude.json");
    let harness = linked(on(&[]), &[goose_folder(), claude_json.clone()]);

    for reads in [Reads::Project, Reads::Anywhere] {
        let config = built(&harness, &[], reads);

        for refused in [goose_folder(), claude_json.clone()] {
            assert!(
                !allows(&config, &text(&refused)),
                "{refused:?} was opened as a global read with {reads:?}"
            );
        }
    }
}
