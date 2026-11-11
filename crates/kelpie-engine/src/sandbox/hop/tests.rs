use super::*;
use std::ffi::OsStr;

/// A profiles folder holding `names`, for one test
fn profiles_folder(names: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for name in names {
        fs::write(dir.path().join(name), "").unwrap();
    }
    dir
}

#[test]
fn a_loaded_kelpie_profile_is_found() {
    let dir = profiles_folder(&["bwrap.4", "kelpie.12", "kelpie_bwrap.13"]);
    assert!(loaded_in(dir.path()));
}

#[test]
fn other_profiles_are_not_kelpies() {
    let dir = profiles_folder(&["kelpie_bwrap.3"]);
    assert!(!loaded_in(dir.path()), "bwrap's profile is not kelpie's");

    let dir = profiles_folder(&["kelpie.", "kelpie.x"]);
    assert!(!loaded_in(dir.path()), "the name after the dot is digits");

    let dir = profiles_folder(&[]);
    assert!(!loaded_in(dir.path()), "no folder entry is no profile");
}

#[test]
fn an_unconfined_app_with_the_profile_loaded_hops() {
    assert!(needed(
        "unconfined\n",
        ["bwrap.4", "kelpie.12", "kelpie_bwrap.13"]
    ));
}

#[test]
fn a_confined_app_does_not_hop() {
    assert!(!needed(
        "kelpie (enforce)\n",
        ["kelpie.12", "kelpie_bwrap.13"]
    ));
}

#[test]
fn no_loaded_kelpie_profile_does_not_hop() {
    assert!(!needed(
        "unconfined\n",
        ["bwrap.4", "kelpie_bwrap.13", "kelpie_sandboxed.14"]
    ));
    assert!(!needed("unconfined\n", Vec::<String>::new()));
}

#[test]
fn the_hop_keeps_the_program_arguments_directory_and_environment() {
    let mut command = Command::new("/usr/bin/bwrap");
    command
        .args(["--die-with-parent", "--", "claude"])
        .current_dir("/work/kelpie")
        .env("KELPIE_RUN", "run-7")
        .env_remove("SECRET");

    let hopped = wrap(Path::new("/usr/bin/kelpie-app"), &command);

    assert_eq!(hopped.get_program(), "/usr/bin/kelpie-app");
    let args: Vec<_> = hopped.get_args().collect();
    assert_eq!(
        args,
        [
            "--sandbox-hop",
            "--",
            "/usr/bin/bwrap",
            "--die-with-parent",
            "--",
            "claude"
        ]
    );
    assert_eq!(hopped.get_current_dir(), Some(Path::new("/work/kelpie")));
    let envs: Vec<_> = hopped.get_envs().collect();
    assert_eq!(
        envs,
        [
            (OsStr::new("KELPIE_RUN"), Some(OsStr::new("run-7"))),
            (OsStr::new("SECRET"), None),
        ]
    );
}

#[test]
fn the_hop_flag_names_the_command_to_exec() {
    let args: Vec<OsString> = ["kelpie-app", "--sandbox-hop", "--", "bwrap", "--", "claude"]
        .map(OsString::from)
        .to_vec();

    let (program, rest) = target(&args).unwrap();

    assert_eq!(program, "bwrap");
    assert_eq!(rest, ["--", "claude"]);
    assert!(target(&args[..1]).is_none());
    assert!(target(&["kelpie-app", "--sandbox-hop"].map(OsString::from)).is_none());
}
