use std::fs;
use std::path::{Path, PathBuf};

use crabpot::Config;
use serde_json::json;

use super::*;
use crate::session::Harness;
use crate::tools::Door;

const PROJECT: &str = "/work/kelpie";
const BESIDE: &str = "/work";
const SESSIONS: &str = "/state/projects";
const STATE: &str = "/state/projects/pm";
const INSTALL: &str = "/opt/claude/bin";

/// A harness whose model answers on `hosts`
fn on(hosts: &[&str]) -> Harness {
    let mut harness = Harness::new("agent");
    harness.hosts = hosts.iter().map(|host| (*host).to_owned()).collect();
    harness
}

/// A harness installed in `folders`, which the session reads
fn installed(mut harness: Harness, folders: &[impl AsRef<Path>]) -> Harness {
    harness.installs = folders
        .iter()
        .map(|folder| folder.as_ref().to_path_buf())
        .collect();
    harness
}

fn project<'a>(allowlist: &'a [String], reads: Reads) -> Project<'a> {
    Project {
        folder: Path::new(PROJECT),
        network_allowlist: allowlist,
        reads,
        sessions_root: Path::new(SESSIONS),
    }
}

/// The settings as crabpot reads them, from the same keys the policy is built on
fn read(settings: serde_json::Value) -> Config {
    let text = settings.to_string();
    Config::from_json(&text).unwrap_or_else(|error| panic!("{error}: {text}"))
}

/// What crabpot makes of `paths` as a list of its own
fn entries(paths: &[&str]) -> Config {
    read(json!({
        "network": {"allowedDomains": [], "deniedDomains": []},
        "filesystem": {
            "denyRead": paths, "allowRead": paths, "allowWrite": paths, "denyWrite": [],
        },
    }))
}

fn built(harness: &Harness, allowlist: &[String], reads: Reads) -> Config {
    policy(harness, &project(allowlist, reads), Path::new(STATE)).unwrap()
}

/// The folder this machine calls home, which the policy closes over
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .expect("the OS names a home folder")
        .into()
}

/// Whether `config` denies `path` whole, as crabpot reads it
fn denies(config: &Config, path: &str) -> bool {
    let wanted = entries(&[path]).filesystem.deny_read;
    config.filesystem.deny_read.contains(&wanted[0])
}

/// Whether `config` denies `path` as a place to write
fn writes_denied(config: &Config, path: &str) -> bool {
    let wanted = read(json!({
        "network": {"allowedDomains": [], "deniedDomains": []},
        "filesystem": {"denyRead": [], "allowRead": [], "allowWrite": [], "denyWrite": [path]},
    }))
    .filesystem
    .deny_write;
    config.filesystem.deny_write.contains(&wanted[0])
}

/// Whether `config` opens `path` back inside what it closes
fn allows(config: &Config, path: &str) -> bool {
    let wanted = entries(&[path]).filesystem.allow_read.unwrap();
    let allowed = config.filesystem.allow_read.as_ref().unwrap();
    allowed.iter().any(|entry| wanted.contains(entry))
}

#[test]
fn read_anywhere_still_denies_every_harness_login() {
    let logins = [
        "~/.claude/.credentials.json",
        "~/.claude.json",
        "~/.codex/**",
        "~/.pi/**",
        "~/.config/goose/**",
        "~/.local/share/goose/**",
        "~/.local/state/goose/**",
        "~/Library/Application Support/Block/goose/**",
    ];

    // Whichever harness the session runs on, and whatever the project reads
    for harness in [on(&["api.anthropic.com"]), on(&["localhost"])] {
        for reads in [Reads::Project, Reads::Anywhere] {
            let config = built(&harness, &[], reads);
            for login in logins {
                assert!(
                    denies(&config, login),
                    "{login:?} is readable with {reads:?}"
                );
            }
        }
    }
}

#[test]
fn the_policy_denies_the_secrets_a_tool_keeps_in_the_home_folder() {
    let secrets = [
        "~/.ssh/**",
        "~/.aws/**",
        "~/.gnupg/**",
        "~/.docker/**",
        "~/.config/gh/**",
        "~/.netrc",
        "~/.git-credentials",
        "~/.npmrc",
        "~/.cargo/credentials",
        "~/.cargo/credentials.toml",
        "~/Library/Keychains/**",
    ];

    for reads in [Reads::Project, Reads::Anywhere] {
        let config = built(&on(&[]), &[], reads);
        for secret in secrets {
            assert!(
                denies(&config, secret),
                "{secret:?} is readable with {reads:?}"
            );
        }
    }
}

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

/// The home folder, the folder holding it, and the folder holding every
/// session's state, each reachable through a link as well as by name
///
/// As `/home` linked to `/data/home` spells one folder two ways, `/home/<name>`
/// and `/data/home/<name>`. The scratch folder is removed when this drops, links
/// included: a link is removed as the name it is, not as what it names.
#[cfg(unix)]
struct Linked {
    scratch: PathBuf,
}

#[cfg(unix)]
impl Linked {
    fn new(test: &str) -> Self {
        let scratch = std::env::temp_dir().join(format!(
            "kelpie-sandbox-linked-{test}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&scratch);
        for folder in ["real/projects", "project"] {
            fs::create_dir_all(scratch.join(folder)).expect("the scratch folders are made");
        }
        for (link, named) in [
            ("home", home()),
            ("above", above_home()),
            ("state", scratch.join("real")),
        ] {
            std::os::unix::fs::symlink(named, scratch.join(link)).expect("the link is made");
        }
        Self { scratch }
    }

    /// The home folder, named through a link
    fn home(&self) -> PathBuf {
        self.scratch.join("home")
    }

    /// The folder holding the home folder, named through a link
    fn above(&self) -> PathBuf {
        self.scratch.join("above")
    }

    /// The folder holding every session's state, named through a link
    fn sessions_root(&self) -> PathBuf {
        self.scratch.join("state/projects")
    }

    /// That same folder, named as the OS names it
    fn real_sessions_root(&self) -> PathBuf {
        self.scratch.join("real/projects")
    }

    /// A folder of its own, holding nothing the policy closes
    fn project(&self) -> PathBuf {
        self.scratch.join("project")
    }
}

#[cfg(unix)]
impl Drop for Linked {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.scratch);
    }
}

/// The folder holding the home folder, such as `/Users`
fn above_home() -> PathBuf {
    home()
        .parent()
        .expect("the OS puts the home folder in one")
        .to_path_buf()
}

/// The topmost folder holding the home folder, which is `/` on macOS and Linux
fn root_of_home() -> PathBuf {
    let mut folder = home();
    while let Some(parent) = folder.parent() {
        folder = parent.to_path_buf();
    }
    folder
}

/// A session whose project folder is `wanted` is refused, naming it
///
/// Its state folder is under the machine's temp folder, so a refusal that came
/// too late would leave it behind.
fn refuses(name: &str, wanted: &Path, reads: Reads) {
    let scratch = std::env::temp_dir()
        .join("kelpie-sandbox-refused")
        .join(name);
    let _ = fs::remove_dir_all(&scratch);
    let (sessions_root, state) = (scratch.join("projects"), scratch.join("projects/pm"));
    let allowlist: Vec<String> = Vec::new();
    let project = Project {
        folder: wanted,
        network_allowlist: &allowlist,
        reads,
        sessions_root: &sessions_root,
    };

    let error = Sandbox::start(&on(&[]), &project, &state)
        .expect_err("a project that holds the home folder is refused");
    let (wanted, message) = (wanted.to_string_lossy().into_owned(), error.to_string());

    let SandboxError::ProjectAtHome { folder } = &error else {
        panic!("the refusal is not the project folder's, with {reads:?}: {error}");
    };
    assert_eq!(folder.to_string_lossy(), wanted, "{reads:?}");
    assert!(
        message.contains(&wanted) && message.contains("move the project into a folder of its own"),
        "the refusal names neither the folder nor what to do: {message}"
    );
    assert!(
        !state.exists(),
        "the state folder was made, so the session started with {reads:?}: {state:?}"
    );
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn a_project_at_the_home_folder_is_refused() {
    let home = home();

    for reads in [Reads::Project, Reads::Anywhere] {
        refuses("at-home", &home, reads);
    }
}

#[test]
fn a_project_above_the_home_folder_is_refused() {
    let above = above_home();

    for reads in [Reads::Project, Reads::Anywhere] {
        refuses("above-home", &above, reads);
    }
}

#[test]
fn a_project_at_the_root_is_refused() {
    let root = root_of_home();
    if cfg!(unix) {
        assert_eq!(root, PathBuf::from("/"), "the root the refusal names");
    }

    for reads in [Reads::Project, Reads::Anywhere] {
        refuses("at-root", &root, reads);
    }
}

/// A project that reaches the home folder through a link is the home folder
///
/// `Sandbox::start` compares the folder a project really names, as crabpot
/// matches it, and not the folder as it was spelled.
#[cfg(unix)]
#[test]
fn a_project_that_links_to_the_home_folder_is_refused() {
    let linked = Linked::new("at-home");

    for reads in [Reads::Project, Reads::Anywhere] {
        refuses("links-to-home", &linked.home(), reads);
    }
}

#[cfg(unix)]
#[test]
fn a_project_that_links_to_the_folder_holding_home_is_refused() {
    let linked = Linked::new("above-home");

    for reads in [Reads::Project, Reads::Anywhere] {
        refuses("links-above-home", &linked.above(), reads);
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

#[test]
fn the_network_is_the_harness_host_plus_the_allowlist() {
    let allowlist = ["docs.rs".to_owned(), "*.crates.io".to_owned()];

    let config = built(
        &on(&["api.anthropic.com", "localhost"]),
        &allowlist,
        Reads::Project,
    );

    let hosts: Vec<&str> = config
        .network
        .allowed_domains
        .iter()
        .map(|entry| entry.as_str())
        .collect();
    assert_eq!(
        hosts,
        ["api.anthropic.com", "localhost", "docs.rs", "*.crates.io"]
    );
    assert!(config.network.denied_domains.is_empty());
    assert_eq!(
        config.network.strict_allowlist,
        Some(true),
        "a host off the list is refused, not asked about"
    );
}

#[test]
fn a_host_the_harness_and_the_allowlist_both_name_is_listed_once() {
    let allowlist = ["api.anthropic.com".to_owned()];

    let config = built(&on(&["api.anthropic.com"]), &allowlist, Reads::Project);

    assert_eq!(config.network.allowed_domains.len(), 1);
}

#[test]
fn a_domain_crabpot_cannot_read_is_refused_naming_its_place() {
    let allowlist = ["not a domain".to_owned()];

    let error = policy(
        &on(&[]),
        &project(&allowlist, Reads::Project),
        Path::new(STATE),
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("allowedDomains.0"),
        "the refusal names the entry's place: {error}"
    );
}

/// The settings with `door` named, as crabpot reads them
fn with_door(door: &Door, reads: Reads) -> Config {
    policy_with_door(
        &on(&["api.anthropic.com"]),
        &project(&[], reads),
        Path::new(STATE),
        Some(door),
    )
    .unwrap()
}

#[test]
fn a_socket_door_is_the_one_unix_socket_the_policy_allows() {
    let door = Door::parse("/data/kelpie/0123.sock").unwrap();

    let config = with_door(&door, Reads::Project);

    let wanted = read(json!({
        "network": {
            "allowedDomains": [], "deniedDomains": [],
            "allowUnixSockets": ["/data/kelpie/0123.sock"],
        },
        "filesystem": {"denyRead": [], "allowWrite": [], "denyWrite": []},
    }));
    assert_eq!(
        config.network.allow_unix_sockets,
        wanted.network.allow_unix_sockets
    );
    assert_eq!(config.network.allow_tcp, None, "no TCP grant beside it");
}

/// Crabpot refuses `allowTcp` where Seatbelt cannot narrow it to one host
#[cfg(target_os = "linux")]
#[test]
fn a_tcp_door_is_the_one_address_the_policy_allows() {
    let door = Door::parse("127.0.0.1:41999").unwrap();

    let config = with_door(&door, Reads::Project);

    let wanted = read(json!({
        "network": {
            "allowedDomains": [], "deniedDomains": [],
            "allowTcp": ["127.0.0.1:41999"],
        },
        "filesystem": {"denyRead": [], "allowWrite": [], "denyWrite": []},
    }));
    assert_eq!(config.network.allow_tcp, wanted.network.allow_tcp);
    assert_eq!(config.network.allow_unix_sockets, None);
}

#[test]
fn a_door_does_not_open_local_binding() {
    let door = if cfg!(target_os = "linux") {
        "127.0.0.1:41999"
    } else {
        "/data/kelpie/0123.sock"
    };

    let config = with_door(&Door::parse(door).unwrap(), Reads::Project);

    assert_eq!(config.network.allow_local_binding, None);
}

#[test]
fn no_door_grants_no_socket_and_no_address() {
    let config = built(&on(&[]), &[], Reads::Project);

    assert_eq!(config.network.allow_unix_sockets, None);
    assert_eq!(config.network.allow_tcp, None);
}

#[test]
fn the_relay_binary_is_readable_when_the_home_folder_is_closed() {
    let door = Door::parse(if cfg!(target_os = "linux") {
        "127.0.0.1:41999"
    } else {
        "/data/kelpie/0123.sock"
    })
    .unwrap();
    let exe = std::env::current_exe().unwrap();

    let config = with_door(&door, Reads::Project);

    assert!(
        config.filesystem.allow_read.unwrap().contains(
            &entries(&[&exe.to_string_lossy()])
                .filesystem
                .allow_read
                .unwrap()[0]
        ),
        "the harness starts it inside the sandbox"
    );
}
