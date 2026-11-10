//! What a harness may read beside its own program: the hosts it talks to, the
//! folder its install sits in, and the folder of the interpreter its script
//! names
//!
//! `harness::Harness`'s `hosts` and `installs`

use super::*;

#[test]
fn claude_names_its_model_host_its_install_and_the_mode_that_stops_its_prompts() {
    let dir = installed();
    let logins = logged_in();

    let harness = build(&path_of([dir.clone()]), &logins).unwrap();

    assert_eq!(harness.hosts, ["api.anthropic.com"]);
    assert!(
        harness.installs.contains(&dir),
        "the folder claude sits in is read: {:?}",
        harness.installs
    );
    assert_eq!(harness.unprompted.as_deref(), Some("bypassPermissions"));
}

#[test]
fn goose_names_the_host_of_its_providers_address() {
    let dir = goose_installed();
    let path = path_of([dir.clone()]);

    for (address, host) in [
        ("http://127.0.0.1:11434", "127.0.0.1"),
        ("http://localhost:11434/v1", "localhost"),
        ("https://models.example.com", "models.example.com"),
        (
            "http://user:secret@gateway.test:8080/path?q=1",
            "gateway.test",
        ),
        ("http://[::1]:11434", "[::1]"),
        ("localhost", "localhost"),
    ] {
        let harness = build_on(&path, &provider("ollama", Some(address))).unwrap();
        assert_eq!(harness.hosts, [host], "{address}");
    }
}

#[test]
fn goose_with_no_address_names_the_default_host_of_a_provider_it_knows() {
    let dir = goose_installed();

    for (name, host) in [
        ("anthropic", "api.anthropic.com"),
        ("openai", "api.openai.com"),
        ("ollama", "localhost"),
    ] {
        let harness = build_on(&path_of([dir.clone()]), &provider(name, None)).unwrap();

        assert_eq!(harness.hosts, [host], "{name}");
    }
}

#[test]
fn goose_with_no_address_on_a_provider_it_does_not_know_names_no_host_and_still_its_install_and_mode()
 {
    let dir = goose_installed();

    let harness = build_on(&path_of([dir.clone()]), &provider("acme", None)).unwrap();

    assert!(harness.hosts.is_empty());
    assert!(harness.installs.contains(&dir), "{:?}", harness.installs);
    assert_eq!(harness.unprompted.as_deref(), Some("auto"));
}

#[test]
fn an_address_the_person_gave_wins_over_the_providers_default_host() {
    let dir = goose_installed();

    let harness = build_on(
        &path_of([dir]),
        &provider("openai", Some("https://proxy.example.com:8443/v1")),
    )
    .unwrap();

    assert_eq!(harness.hosts, ["proxy.example.com"]);
}

#[test]
fn an_address_with_no_host_names_none() {
    let dir = goose_installed();

    let harness = build_on(&path_of([dir]), &provider("ollama", Some("http://:11434"))).unwrap();

    assert!(harness.hosts.is_empty());
}

#[test]
fn an_address_with_a_bare_host_name_names_none_for_the_sandbox_refuses_one() {
    let dir = goose_installed();

    for address in [
        "http://ollama:11434",
        "http://gateway/v1",
        "http://host.:80",
    ] {
        let harness =
            build_on(&path_of([dir.clone()]), &provider("ollama", Some(address))).unwrap();

        assert!(harness.hosts.is_empty(), "{address}: {:?}", harness.hosts);
    }
}

/// A runnable file `name` in `dir` holding `text`
#[cfg(unix)]
fn script(dir: &Path, name: &str, text: &str) -> PathBuf {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt as _;
    let file = dir.join(name);
    fs::write(&file, text).unwrap();
    fs::set_permissions(&file, Permissions::from_mode(0o755)).unwrap();
    file
}

#[cfg(unix)]
#[test]
fn a_script_run_by_env_opens_the_folder_of_the_interpreter_it_names() {
    // Claude Code from npm under nvm: a link in `bin/` to a `cli.js` that starts
    // with `#!/usr/bin/env node`, and a `node` in a folder of its own on the PATH
    let package = temp("harness-package");
    let cli = script(&package, "cli.js", "#!/usr/bin/env node\nconsole.log(1)\n");
    let bin = temp("harness-bin");
    std::os::unix::fs::symlink(&cli, bin.join("claude")).unwrap();
    let node_dir = temp("harness-node");
    script(&node_dir, "node", "#!/bin/sh\n");

    let harness = build(&path_of([bin.clone(), node_dir.clone()]), &logged_in()).unwrap();

    for folder in [&bin, &package, &node_dir] {
        assert!(
            harness.installs.contains(folder),
            "{folder:?} is read: {:?}",
            harness.installs
        );
    }
}

#[cfg(unix)]
#[test]
fn an_interpreter_named_by_its_path_and_one_behind_a_flag_are_opened_too() {
    let interpreters = temp("harness-interpreters");
    script(&interpreters, "tool", "#!/bin/sh\n");
    let node_dir = temp("harness-node");
    script(&node_dir, "node", "#!/bin/sh\n");
    let by_path = temp("harness-by-path");
    script(
        &by_path,
        "claude",
        &format!("#!{} -e\n", interpreters.join("tool").display()),
    );
    let by_flag = temp("harness-by-flag");
    script(
        &by_flag,
        "claude",
        "#!/usr/bin/env -S NODE_ENV=x node --no-warnings\n",
    );

    let by_path_harness = build(&path_of([by_path]), &logged_in()).unwrap();
    let by_flag_harness = build(&path_of([by_flag, node_dir.clone()]), &logged_in()).unwrap();

    assert!(by_path_harness.installs.contains(&interpreters));
    assert!(by_flag_harness.installs.contains(&node_dir));
}

#[cfg(unix)]
#[test]
fn an_interpreter_that_is_not_on_the_path_adds_no_folder() {
    let bin = temp("harness-bin");
    script(&bin, "claude", "#!/usr/bin/env nodeless\n");

    let harness = build(&path_of([bin.clone()]), &logged_in()).unwrap();

    assert!(harness.installs.contains(&bin));
}
