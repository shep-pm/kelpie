use std::path::Path;

use crabpot::Config;
use serde_json::json;

use super::*;
use crate::session::Harness;

const PROJECT: &str = "/work/kelpie";
const WORKTREE: &str = "/data/kelpie/worktrees/74";
const GIT_DIR: &str = "/data/kelpie/worktrees/74.git";
const COMMON_DIR: &str = "/work/kelpie/.git";
const STATE: &str = "/data/kelpie/state/74";
const KELPIE_HOME: &str = "/data/kelpie";
const SESSIONS: &str = "/data/kelpie/projects";
const LEASE_ADDRESS: &str = "127.0.0.1:41998";

/// A harness whose model answers on `hosts`
fn on(hosts: &[&str]) -> Harness {
    let mut harness = Harness::new("agent");
    harness.hosts = hosts.iter().map(|host| (*host).to_owned()).collect();
    harness
}

fn project<'a>(allowlist: &'a [String], read_anywhere: bool) -> Project<'a> {
    Project {
        folder: Path::new(PROJECT),
        network_allowlist: allowlist,
        reads: if read_anywhere {
            Reads::Anywhere
        } else {
            Reads::Project
        },
        sessions_root: Path::new(SESSIONS),
    }
}

/// The lease door of the worker under test: its state folder's socket, or on
/// Linux the loopback address the same door names
fn lease() -> &'static Door {
    static HELD: std::sync::OnceLock<Door> = std::sync::OnceLock::new();
    HELD.get_or_init(|| {
        let text = if cfg!(target_os = "linux") {
            LEASE_ADDRESS.to_owned()
        } else {
            format!("{STATE}/{LEASE_SOCKET}")
        };
        Door::parse(&text).unwrap()
    })
}

fn worker(door: Option<&Door>) -> Worker<'_> {
    Worker {
        worktree: Path::new(WORKTREE),
        git_dir: Path::new(GIT_DIR),
        git_common_dir: Path::new(COMMON_DIR),
        state_dir: Path::new(STATE),
        kelpie_home: Path::new(KELPIE_HOME),
        door,
        lease: Some(lease()),
    }
}

fn built(harness: &Harness, allowlist: &[String], read_anywhere: bool) -> Config {
    policy(harness, &project(allowlist, read_anywhere), &worker(None)).unwrap()
}

/// What crabpot makes of `paths` as a list of its own
fn entries(paths: &[&str]) -> Config {
    let settings = json!({
        "network": {"allowedDomains": [], "deniedDomains": []},
        "filesystem": {
            "denyRead": paths, "allowRead": paths, "allowWrite": paths, "denyWrite": paths,
        },
    });
    Config::from_json(&settings.to_string()).unwrap()
}

#[test]
fn the_worktree_its_git_dir_and_the_state_folder_are_the_only_writes() {
    let mut harness = on(&["api.anthropic.com"]);
    harness.installs = vec!["/opt/claude".into()];

    for read_anywhere in [false, true] {
        let config = built(&harness, &["docs.rs".to_owned()], read_anywhere);

        assert_eq!(
            config.filesystem.allow_write,
            entries(&[WORKTREE, GIT_DIR, STATE]).filesystem.allow_write,
            "with read_anywhere {read_anywhere}"
        );
    }
}

#[test]
fn the_clones_hooks_and_config_are_not_writable() {
    let config = built(&on(&[]), &[], false);

    let denied = entries(&[
        "/data/kelpie/worktrees/74.git/config",
        "/data/kelpie/worktrees/74.git/hooks",
        "/data/kelpie/worktrees/74.git/info",
        "/data/kelpie/worktrees/74.git/modules",
        "/data/kelpie/worktrees/74.git/objects/info",
    ])
    .filesystem
    .deny_write;
    for entry in &denied {
        assert!(
            config.filesystem.deny_write.contains(entry),
            "{entry:?} is writable"
        );
    }
}

#[test]
fn a_harnesss_own_files_in_the_worktree_are_not_writable() {
    let config = built(&on(&[]), &[], false);

    // Claude Code's, goose's and Codex's, whichever harness the worker runs on
    let denied = entries(&[
        "/data/kelpie/worktrees/74/.claude",
        "/data/kelpie/worktrees/74/.mcp.json",
        "/data/kelpie/worktrees/74/.goose",
        "/data/kelpie/worktrees/74/.codex",
    ])
    .filesystem
    .deny_write;
    for entry in &denied {
        assert!(
            config.filesystem.deny_write.contains(entry),
            "{entry:?} is writable"
        );
    }
}

/// Three entries of the person's own global folder a harness links into the
/// worker's config folder, and the folder they sit in
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
fn a_linked_entry_of_the_persons_global_folder_is_read_and_never_written() {
    let harness = linked(
        on(&[]),
        &[THEIR_CLAUDE_MD, THEIR_SKILLS, THEIR_GOOSE_CONFIG],
    );

    for read_anywhere in [false, true] {
        let config = built(&harness, &[], read_anywhere);
        let (allowed, denied) = (
            config.filesystem.allow_read.unwrap(),
            entries(&[THEIR_CLAUDE_MD, THEIR_SKILLS, THEIR_GOOSE_CONFIG])
                .filesystem
                .deny_write,
        );

        for (entry, path) in denied
            .iter()
            .zip([THEIR_CLAUDE_MD, THEIR_SKILLS, THEIR_GOOSE_CONFIG])
        {
            assert!(
                allowed.contains(entry),
                "the worker cannot read the file its harness linked in at {path:?} with read_anywhere {read_anywhere}"
            );
            assert!(
                config.filesystem.deny_write.contains(entry),
                "the worker could write through the link at {path:?} with read_anywhere {read_anywhere}"
            );
        }
        for folder in [THEIR_HOME, "/home/person/.claude"] {
            let named = entries(&[folder]).filesystem.allow_read.unwrap()[0].clone();
            assert!(
                !allowed.contains(&named),
                "the person's global folder was opened whole, and not just what was linked into the worker's config folder: {named:?}"
            );
        }
        assert_eq!(
            config.filesystem.allow_write,
            entries(&[WORKTREE, GIT_DIR, STATE]).filesystem.allow_write,
            "a worker could write outside its worktree, git dir and state folder"
        );
    }
}

#[test]
fn a_workers_gooses_config_is_read_and_the_secrets_beside_it_stay_denied() {
    // Under the real home folder, where the policy's login denial really applies
    let folder = home()
        .expect("the OS names a home folder")
        .join(".config/goose");
    let config_yaml = text(&folder.join("config.yaml"));
    let harness = linked(on(&[]), &[folder.join("config.yaml")]);

    for read_anywhere in [false, true] {
        let config = built(&harness, &[], read_anywhere);
        let allowed = config.filesystem.allow_read.unwrap();

        let opened =
            |path: &str| allowed.contains(&entries(&[path]).filesystem.allow_read.unwrap()[0]);
        assert!(
            opened(&config_yaml),
            "goose's config.yaml is not read with read_anywhere {read_anywhere}"
        );
        for closed in [text(&folder.join("secrets.yaml")), text(&folder)] {
            assert!(
                !opened(&closed),
                "{closed:?} was opened with read_anywhere {read_anywhere}"
            );
        }
        assert!(
            config
                .filesystem
                .deny_read
                .contains(&entries(&["~/.config/goose/**"]).filesystem.deny_read[0]),
            "goose's config folder is no longer denied with read_anywhere {read_anywhere}"
        );
    }
}

/// The policy for a person whose home folder is `home`, with `vars` as the
/// environment their tools read
fn at(home: &Path, vars: &[(&'static str, String)], read_anywhere: bool) -> Config {
    let env = vars
        .iter()
        .cloned()
        .collect::<std::collections::BTreeMap<_, _>>();
    compose(
        &on(&["api.anthropic.com"]),
        &project(&[], read_anywhere),
        &worker(None),
        Some(home),
        &env,
    )
    .unwrap()
}

#[test]
fn the_maintainers_logins_and_cargo_credentials_are_denied() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let moved = home.join("builds/cargo");
    std::fs::create_dir_all(&moved).unwrap();
    std::fs::create_dir_all(home.join(".cargo")).unwrap();
    let vars = [("CARGO_HOME", moved.to_string_lossy().into_owned())];
    let (default, moved) = (home.join(".cargo"), moved.to_string_lossy().into_owned());
    let mut wanted = vec![
        "~/.claude/.credentials.json".to_owned(),
        "~/.claude.json".to_owned(),
        "~/.codex/**".to_owned(),
        "~/.config/goose/**".to_owned(),
        "~/.ssh/**".to_owned(),
        "~/.config/gh/**".to_owned(),
        "~/.git-credentials".to_owned(),
        "~/.cargo/credentials".to_owned(),
        "~/.cargo/credentials.toml".to_owned(),
        "~/.kelpie/projects/**".to_owned(),
        "~/.kelpie/settings.toml".to_owned(),
        "~/.kelpie/totp/**".to_owned(),
    ];
    // A cargo home moved elsewhere keeps its own, and so does the default one
    for cargo in [moved.as_str(), &default.to_string_lossy()] {
        wanted.push(format!("{cargo}/credentials"));
        wanted.push(format!("{cargo}/credentials.toml"));
    }
    let wanted: Vec<&str> = wanted.iter().map(String::as_str).collect();

    for read_anywhere in [false, true] {
        let config = at(home, &vars, read_anywhere);
        for entry in &entries(&wanted).filesystem.deny_read {
            assert!(
                config.filesystem.deny_read.contains(entry),
                "{entry:?} is readable with read_anywhere {read_anywhere}"
            );
        }
    }
}

#[test]
fn the_home_folder_and_kelpies_data_folder_are_closed_and_what_the_worker_works_in_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir_all(home.join(".rustup")).unwrap();
    let mut harness = on(&[]);
    harness.installs = vec!["/opt/claude/bin".into()];
    let env = std::collections::BTreeMap::new();

    let config = compose(
        &harness,
        &project(&[], false),
        &worker(None),
        Some(home),
        &env,
    )
    .unwrap();

    let exe = std::env::current_exe().unwrap();
    let rustup = home.join(".rustup");
    let wanted = entries(&[
        WORKTREE,
        GIT_DIR,
        COMMON_DIR,
        PROJECT,
        "/work",
        STATE,
        "/opt/claude/bin",
        &exe.to_string_lossy(),
        &rustup.to_string_lossy(),
    ]);
    assert_eq!(config.filesystem.allow_read, wanted.filesystem.allow_read);
    let closed = entries(&[
        &format!("{}/**", home.display()),
        &format!("{KELPIE_HOME}/**"),
    ]);
    for entry in &closed.filesystem.deny_read {
        assert!(config.filesystem.deny_read.contains(entry), "{entry:?}");
    }
}

#[test]
fn a_project_that_reads_anywhere_still_closes_kelpies_data_folder() {
    let dir = tempfile::tempdir().unwrap();

    let config = at(dir.path(), &[], true);

    let kelpie = entries(&[&format!("{KELPIE_HOME}/**")])
        .filesystem
        .deny_read;
    assert!(config.filesystem.deny_read.contains(&kelpie[0]));
    let home = entries(&[&format!("{}/**", dir.path().display())])
        .filesystem
        .deny_read;
    assert!(
        !config.filesystem.deny_read.contains(&home[0]),
        "the home folder stays open"
    );
}

#[test]
fn github_is_reached_and_no_host_beyond_the_harness_and_the_allowlist() {
    let allowlist = ["docs.rs".to_owned(), "github.com".to_owned()];

    let config = built(&on(&["api.anthropic.com"]), &allowlist, false);

    let hosts: Vec<&str> = config
        .network
        .allowed_domains
        .iter()
        .map(|entry| entry.as_str())
        .collect();
    assert_eq!(
        hosts,
        [
            "api.anthropic.com",
            "docs.rs",
            "github.com",
            "api.github.com"
        ]
    );
    assert!(config.network.denied_domains.is_empty());
    assert_eq!(config.network.strict_allowlist, Some(true));
}

/// The settings with only `network` named, as crabpot reads them
fn network(network: serde_json::Value) -> Config {
    let mut network = network;
    network["allowedDomains"] = json!([]);
    network["deniedDomains"] = json!([]);
    Config::from_json(
        &json!({
            "network": network,
            "filesystem": {"denyRead": [], "allowWrite": [], "denyWrite": []},
        })
        .to_string(),
    )
    .unwrap()
}

#[cfg(not(target_os = "linux"))]
#[test]
fn the_lease_socket_and_the_door_are_the_only_sockets() {
    let door = Door::parse("/data/kelpie/0123.sock").unwrap();

    let with = policy(&on(&[]), &project(&[], false), &worker(Some(&door))).unwrap();
    let without = built(&on(&[]), &[], false);

    let wanted = network(json!({
        "allowUnixSockets": ["/data/kelpie/state/74/lease.sock", "/data/kelpie/0123.sock"],
    }));
    assert_eq!(
        with.network.allow_unix_sockets,
        wanted.network.allow_unix_sockets
    );
    assert_eq!(
        without.network.allow_unix_sockets,
        network(json!({"allowUnixSockets": ["/data/kelpie/state/74/lease.sock"]}))
            .network
            .allow_unix_sockets,
        "the lease socket alone, with no door"
    );
    assert_eq!(with.network.allow_tcp, None);
    assert_eq!(with.network.allow_local_binding, None);
}

/// Linux turns every Unix socket off, so the door and the lease are addresses
#[cfg(target_os = "linux")]
#[test]
fn the_lease_socket_and_the_door_are_the_only_sockets() {
    let door = Door::parse("127.0.0.1:41999").unwrap();

    let with = policy(&on(&[]), &project(&[], false), &worker(Some(&door))).unwrap();
    let without = built(&on(&[]), &[], false);

    assert_eq!(
        with.network.allow_tcp,
        network(json!({"allowTcp": ["127.0.0.1:41998", "127.0.0.1:41999"]}))
            .network
            .allow_tcp
    );
    assert_eq!(
        without.network.allow_tcp,
        network(json!({"allowTcp": ["127.0.0.1:41998"]}))
            .network
            .allow_tcp,
        "the lease address alone, with no door"
    );
    assert_eq!(with.network.allow_unix_sockets, None);
    assert_eq!(with.network.allow_local_binding, None);
}
