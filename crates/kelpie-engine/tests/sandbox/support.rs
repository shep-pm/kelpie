//! The scratch world every sandbox test builds on, and the sandbox for it
//!
//! A folder under the target's temp folder with a project, the state folders of two
//! sessions and a folder outside all of them, plus the one way a test runs `sh`
//! inside a sandbox. A child of `tests/sandbox.rs`, which is its own `main`
//! (crabpot's Linux helper is this binary run again).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use kelpie_engine::sandbox::{Project, Reads, Sandbox};
use kelpie_engine::session::Harness;
use kelpie_engine::tools::Door;

/// A folder of its own under the target's temp folder, with a project, the state
/// folders of two sessions, and a folder outside all of them, removed when it
/// drops
pub(super) struct Scratch {
    pub(super) root: PathBuf,
}

impl Scratch {
    pub(super) fn new(test: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("sandbox-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // The project sits one below the folder it may read beside, so the
        // `outside` folder is beside neither it nor the session's state
        for folder in [
            "work/project",
            "outside",
            "data/projects/pm",
            "data/projects/other",
        ] {
            fs::create_dir_all(root.join(folder)).expect("the scratch folders are made");
        }
        // The path as the OS names it, since a sandbox compares real paths
        let root = fs::canonicalize(root).expect("the scratch folder is there");
        Self { root }
    }

    pub(super) fn project(&self) -> PathBuf {
        self.root.join("work/project")
    }

    pub(super) fn state(&self) -> PathBuf {
        self.sessions_root().join("pm")
    }

    /// The folder holding this session's state and another's, as kelpie's data
    /// folder holds one per session
    pub(super) fn sessions_root(&self) -> PathBuf {
        self.root.join("data/projects")
    }

    pub(super) fn other(&self) -> PathBuf {
        self.sessions_root().join("other")
    }

    pub(super) fn outside(&self) -> PathBuf {
        self.root.join("outside")
    }

    /// A folder of this scratch, made when it is asked for
    pub(super) fn folder(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(&path).expect("the scratch folder is made");
        path
    }

    /// A sandbox for a session in the project that reads as `reads`, on `program`,
    /// with kelpie's tools `door` named in its policy when it has one
    pub(super) fn sandbox(&self, program: &Path, reads: Reads, door: Option<&Door>) -> Sandbox {
        sandbox_in(
            program,
            &self.project(),
            &self.state(),
            &self.sessions_root(),
            reads,
            &[],
            door,
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A sandbox for a session whose harness is `program`, working in `project`, keeping
/// its files in `state` under `sessions_root`, reading as `reads`, reading
/// `global_reads` of the person's own global folder, and reaching kelpie's tools
/// `door` when it has one
pub(super) fn sandbox_in(
    program: &Path,
    project: &Path,
    state: &Path,
    sessions_root: &Path,
    reads: Reads,
    global_reads: &[PathBuf],
    door: Option<&Door>,
) -> Sandbox {
    let harness = harness_in(program, global_reads);
    let allowlist: Vec<String> = Vec::new();
    let confined = Project {
        folder: project,
        network_allowlist: &allowlist,
        reads,
        sessions_root,
    };
    Sandbox::start_with_door(&harness, &confined, state, door)
        .unwrap_or_else(|error| panic!("the sandbox did not start: {error}"))
}

/// The harness of a test's session: `program`, whose own install the session reads,
/// reading `global_reads` of the person's own global folder
pub(super) fn harness_in(program: &Path, global_reads: &[PathBuf]) -> Harness {
    let mut harness = Harness::new(program);
    // The harness's own install is read, as a real one's is
    harness.installs = program
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .collect();
    harness.global_reads = global_reads.to_vec();
    harness
}

/// A folder under the home folder, removed when it drops
///
/// The home folder is the area `Reads::Project` closes whole and `Reads::Anywhere`
/// opens, so a file outside the project only follows the setting when it sits
/// inside it. kelpie's own name and this process's id keep the folder from
/// meeting another run's.
pub(super) struct HomeFolder {
    folder: PathBuf,
}

impl HomeFolder {
    pub(super) fn new(test: &str) -> Self {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .expect("the OS names a home folder");
        let folder =
            PathBuf::from(home).join(format!(".kelpie-sandbox-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("the home folder takes a scratch folder for the test");
        Self { folder }
    }

    pub(super) fn file(&self, name: &str) -> PathBuf {
        self.folder.join(name)
    }
}

impl Drop for HomeFolder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

/// `script` run by `sh` inside the sandbox
pub(super) fn sh(sandbox: &Sandbox, script: &str) -> Output {
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    sandbox
        .command(&command)
        .expect("the command goes in the sandbox")
        .output()
        .expect("the sandbox runs sh")
}
