//! The AppArmor profile the `.deb` and `.rpm` ship, and how they ship it
//!
//! Install (`docs/design/sandbox-setup.md`, Install) copies a profile only from a
//! path the package already holds root-owned, so the file and the bundle config
//! are read together here: the rules in the file, the binary path they name, and
//! where each package puts it. The helper script `pkexec` runs and the polkit
//! action that gives its prompt a sentence ship beside it, and are read here too.
//! Nothing here loads the profile.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tauri::utils::config::Config;

/// The profile in the repo, relative to `src-tauri`
const PROFILE_SOURCE: &str = "linux/kelpie.apparmor";

/// Where both packages install it: root-owned, and no folder a person writes
const PROFILE_INSTALLED: &str = "/usr/share/kelpie-app/kelpie.apparmor";

/// The helper script Install runs through `pkexec`, relative to `src-tauri`
const HELPER_SOURCE: &str = "linux/install-profile";

/// Where both packages install the helper, which `sandbox_setup::install::HELPER` runs
const HELPER_INSTALLED: &str = "/usr/libexec/kelpie-app/install-profile";

/// The polkit action file, relative to `src-tauri`
const POLICY_SOURCE: &str = "linux/pm.shep.kelpie.policy";

/// Where both packages install it, in the folder polkit reads actions from
const POLICY_INSTALLED: &str = "/usr/share/polkit-1/actions/pm.shep.kelpie.policy";

/// The system bwrap the profile starts, from the `bubblewrap` package
const BWRAP: &str = "/usr/bin/bwrap";

/// The three profiles, in the order `docs/design/sandbox-setup.md` gives them
const PROFILES: [&str; 3] = ["kelpie", "kelpie_bwrap", "kelpie_sandboxed"];

/// One profile read from the file
struct Profile {
    /// Its `profile ...` line, as written
    header: String,
    /// Its rule lines inside the braces, comments dropped
    rules: Vec<String>,
}

/// The profile file's text
fn profile_text() -> String {
    std::fs::read_to_string(src_tauri().join(PROFILE_SOURCE))
        .expect("the profile the packages ship should be readable")
}

/// `src-tauri`, which holds the profile beside the config that ships it
fn src_tauri() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `src-tauri/tauri.conf.json`, read as Tauri reads it
fn config() -> Config {
    let path = src_tauri().join("tauri.conf.json");
    let text =
        std::fs::read_to_string(&path).expect("src-tauri/tauri.conf.json should be readable");
    serde_json::from_str(&text).expect("src-tauri/tauri.conf.json should read as a Tauri config")
}

/// The file's profiles, each with the rules inside its braces
///
/// An AppArmor comment runs from `#` to the end of its line, and a rule ends at a
/// comma. The `abi` and `include` lines before the first profile are not rules.
fn profiles(text: &str) -> Vec<Profile> {
    let mut read = Vec::new();
    let mut inside = false;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if !inside {
            if line.starts_with("profile ") {
                inside = line.contains('{');
                read.push(Profile {
                    header: line.to_string(),
                    rules: Vec::new(),
                });
            }
            continue;
        }
        if line == "}" {
            inside = false;
            continue;
        }
        read.last_mut()
            .expect("a rule follows the profile it belongs to")
            .rules
            .push(line.strip_suffix(',').unwrap_or(line).to_string());
    }
    read
}

/// A profile's name: the second word of its header
fn name(header: &str) -> &str {
    header.split_whitespace().nth(1).unwrap_or_default()
}

/// The path a profile is attached to, or `None` for one reached only by a transition
fn attached(header: &str) -> Option<&str> {
    header
        .split_whitespace()
        .nth(2)
        .filter(|word| word.starts_with('/'))
}

/// Whether any of `profile`'s rules grants `permission` as a bare word of its own
fn grants(profile: &Profile, permission: &str) -> bool {
    profile.rules.iter().any(|rule| {
        rule.split(',')
            .flat_map(|part| part.split_whitespace())
            .any(|word| word == permission)
    })
}

/// `/usr/bin/<mainBinaryName>`, the path both packages install kelpie's binary at
fn installed_binary() -> String {
    let name = config()
        .main_binary_name
        .expect("tauri.conf.json names the binary the profile points at");
    format!("/usr/bin/{name}")
}

/// `package` installs the profile at `PROFILE_INSTALLED`, the helper at
/// `HELPER_INSTALLED` and the polkit action at `POLICY_INSTALLED`, depends on
/// `bubblewrap`, and runs no script of its own when it is installed
///
/// Tauri's bundler reads each `files` entry as `<path in the package>: <path on
/// disk>`, the disk path relative to `src-tauri`, which it makes the current
/// directory before bundling
/// (`crates/tauri-bundler/src/utils/fs_utils.rs:155`, `.../linux/rpm.rs:231`).
fn ships(
    package: &str,
    files: &HashMap<PathBuf, PathBuf>,
    depends: &Option<Vec<String>>,
    script: &Option<PathBuf>,
) {
    assert_eq!(
        files.get(Path::new(PROFILE_INSTALLED)),
        Some(&PathBuf::from(PROFILE_SOURCE)),
        "the {package} must install the profile at {PROFILE_INSTALLED}, which no person can write"
    );
    assert!(
        src_tauri().join(PROFILE_SOURCE).is_file(),
        "the {package} names {PROFILE_SOURCE} as the file it ships"
    );
    assert_eq!(
        files.get(Path::new(HELPER_INSTALLED)),
        Some(&PathBuf::from(HELPER_SOURCE)),
        "the {package} must install the helper at {HELPER_INSTALLED}, the path the polkit action names"
    );
    assert_eq!(
        files.get(Path::new(POLICY_INSTALLED)),
        Some(&PathBuf::from(POLICY_SOURCE)),
        "the {package} must install the polkit action at {POLICY_INSTALLED}, where polkit reads actions"
    );
    assert!(
        depends.iter().flatten().any(|dep| dep == "bubblewrap"),
        "the {package} must depend on bubblewrap, whose {BWRAP} the profile starts"
    );
    assert!(
        script.is_none(),
        "the {package} must run no install script: loading the profile is kelpie's Install, behind one password prompt"
    );
}

#[test]
fn the_profile_is_three_profiles_and_only_the_second_grants_userns() {
    let read = profiles(&profile_text());
    let names: Vec<&str> = read.iter().map(|profile| name(&profile.header)).collect();
    assert_eq!(
        names, PROFILES,
        "the file is kelpie, bwrap as kelpie started it, and the sandboxed command"
    );

    let granting: Vec<&str> = read
        .iter()
        .filter(|profile| grants(profile, "userns"))
        .map(|profile| name(&profile.header))
        .collect();
    assert_eq!(
        granting,
        ["kelpie_bwrap"],
        "only bwrap as kelpie started it may make a user namespace"
    );
}

#[test]
fn the_profile_names_the_path_the_packages_install_the_binary_at() {
    let binary = installed_binary();
    let read = profiles(&profile_text());

    assert_eq!(
        attached(&read[0].header),
        Some(binary.as_str()),
        "the kelpie profile must be attached to the installed binary, not to a path only a dev build has"
    );
    assert!(
        read[1]
            .rules
            .iter()
            .any(|rule| rule == &format!("{binary} ix")),
        "bwrap as kelpie started it runs kelpie's binary again as its helper, and stays in that profile"
    );
}

#[test]
fn both_packages_ship_the_profile_and_depend_on_bubblewrap() {
    let linux = &config().bundle.linux;
    ships(
        "deb",
        &linux.deb.files,
        &linux.deb.depends,
        &linux.deb.post_install_script,
    );
    ships(
        "rpm",
        &linux.rpm.files,
        &linux.rpm.depends,
        &linux.rpm.post_install_script,
    );
    assert!(
        linux.appimage.files.is_empty(),
        "an AppImage runs from a new mount path each launch, so no profile can name it and it ships none, nor the helper and polkit action that install it"
    );
}

#[test]
fn the_install_helper_runs_the_two_fixed_commands_and_is_executable() {
    let path = src_tauri().join(HELPER_SOURCE);
    let text =
        std::fs::read_to_string(&path).expect("the helper the packages ship should be readable");

    assert!(
        text.starts_with("#!/bin/sh\n"),
        "the helper is a /bin/sh script"
    );
    assert!(
        text.lines().any(|line| line == "set -eu"),
        "the helper stops at the first failure and at an unset variable"
    );
    assert!(
        text.lines().any(|line| line
            == format!(
                "/usr/bin/install -m 0644 {PROFILE_INSTALLED} /etc/apparmor.d/kelpie && /usr/sbin/apparmor_parser -r /etc/apparmor.d/kelpie"
            )),
        "the helper copies the packaged profile and loads it, by absolute paths"
    );
    assert!(
        !text.contains('$'),
        "nothing from the environment or the arguments reaches a command the helper runs as root"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_ne!(
            mode & 0o111,
            0,
            "the helper must be executable, and the bundler keeps the source file's mode"
        );
    }
}

#[test]
fn the_polkit_action_names_the_helper_and_asks_for_a_password() {
    let text = std::fs::read_to_string(src_tauri().join(POLICY_SOURCE))
        .expect("the polkit action the packages ship should be readable");

    assert!(
        text.contains("<action id=\"pm.shep.kelpie.install-profile\">"),
        "one action, named for what it does"
    );
    assert_eq!(
        text.matches("<action ").count(),
        1,
        "the file holds the one action"
    );
    assert!(
        text.contains("<message>") && text.contains("</message>"),
        "the action carries the sentence the password prompt shows"
    );
    assert!(
        text.contains(&format!(
            "<annotate key=\"org.freedesktop.policykit.exec.path\">{HELPER_INSTALLED}</annotate>"
        )),
        "polkit matches the action to pkexec by the helper's path alone"
    );
    for default in ["allow_any", "allow_inactive", "allow_active"] {
        assert!(
            text.contains(&format!("<{default}>auth_admin</{default}>")),
            "{default} asks for an administrator's password"
        );
    }
}

#[test]
fn the_profile_leaves_bwrap_as_a_whole_alone() {
    let text = profile_text();
    let read = profiles(&text);

    for profile in &read {
        assert_ne!(
            attached(&profile.header),
            Some(BWRAP),
            "no profile may be attached to {BWRAP} itself: that is Ubuntu's bwrap-userns-restrict, and it lifts the restriction for every program that calls bwrap"
        );
    }

    let naming: Vec<(&str, &str)> = read
        .iter()
        .flat_map(|profile| {
            profile
                .rules
                .iter()
                .map(|rule| (name(&profile.header), rule.as_str()))
        })
        .filter(|(_, rule)| rule.contains(BWRAP))
        .collect();
    assert_eq!(
        naming,
        [("kelpie", "/usr/bin/bwrap Px -> kelpie_bwrap")],
        "the only rule naming {BWRAP} is kelpie's own transition into kelpie_bwrap"
    );

    assert!(
        !text.contains("sysctl"),
        "the profile sets no sysctl: kelpie never turns kernel.apparmor_restrict_unprivileged_userns off"
    );
}
