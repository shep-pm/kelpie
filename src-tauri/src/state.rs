//! What the app holds between commands: the process tree, the projects and the PM's session
//!
//! [`AppState`] is what Tauri manages. It opens the run file in the app's data
//! folder, sweeping what the previous run left behind before anything starts
//! (ADR 0001), reads `projects.json` from the same folder, and hands the tree
//! the reaper beside the app's executable, which the engine starts as the run's
//! first process so a crash or a `SIGKILL` still stops what the run started.
//!
//! `projects.json` is a list from the start, each entry
//! `{ "id", "path", "pm_session" }`, so the rail can add projects later without
//! a migration. Phase 1 has one: the folder the maintainer picks once. Each
//! project keeps its harness state in `<data_dir>/projects/<id>/pm/`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use kelpie_engine::login::Store;
use kelpie_engine::run::RunId;
use kelpie_engine::session::{Session, SessionId};
use kelpie_engine::tree::{ProcessTree, sweep_leftovers};
use serde::{Deserialize, Serialize};

/// The run file's name in the data folder
const RUN_FILE: &str = "run.json";

/// The list of projects, by name, in the data folder
const PROJECTS_FILE: &str = "projects.json";

/// The folder under the data folder that holds each project's state
const PROJECTS_FOLDER: &str = "projects";

/// The folder inside a project's folder that holds its PM's harness state
const PM_FOLDER: &str = "pm";

/// How long a quit gives each process to stop before the ladder goes on
const STOP_GRACE: Duration = Duration::from_secs(5);

/// The reaper's name, as `bundle.externalBin` in `tauri.conf.json` names it
///
/// Tauri reads that as `src-tauri/binaries/kelpie-reaper-<target triple>`, which
/// `scripts/build-reaper.sh` writes, and copies it beside the app binary under
/// this name, the triple dropped.
const REAPER: &str = "kelpie-reaper";

/// The reaper beside this executable, where Tauri puts a sidecar
///
/// `tauri-build` copies each `bundle.externalBin` next to the app binary with the
/// target triple it requires dropped from the file's name, and every OS bundle
/// puts a sidecar beside the binary it bundles. So the name `tauri.conf.json`
/// gives, beside this executable, is where a run finds its reaper.
///
/// The bare name when the OS cannot say where this executable is, which
/// [`AppState::open`] then refuses while naming the path it looked at.
#[must_use]
pub fn reaper_path() -> PathBuf {
    let name = if cfg!(windows) {
        format!("{REAPER}.exe")
    } else {
        REAPER.to_owned()
    };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|folder| folder.join(&name)))
        .unwrap_or_else(|| PathBuf::from(&name))
}

/// One project kelpie holds a PM for
// wire format: `projects.json` holds these entries
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// A name of kelpie's own for the project, which its state folder carries
    pub id: String,

    /// The folder the maintainer picked
    pub path: PathBuf,

    /// The id the harness gave the PM's session, kept to resume it. `None`
    /// until the first message.
    pub pm_session: Option<SessionId>,
}

/// The projects, and the file that keeps them
#[derive(Debug)]
pub struct Projects {
    file: PathBuf,
    list: Vec<Project>,
}

impl Projects {
    /// The projects `file` holds, none when it is not there yet
    ///
    /// # Errors
    ///
    /// The file cannot be read, or is not a list of projects.
    pub fn load(file: PathBuf) -> io::Result<Self> {
        let list = match fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} is not a list of projects: {error}", file.display()),
                )
            })?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        Ok(Self { file, list })
    }

    /// The projects, in the order they were added
    #[must_use]
    pub fn list(&self) -> &[Project] {
        &self.list
    }

    /// The only project Phase 1 has
    #[must_use]
    pub fn first(&self) -> Option<&Project> {
        self.list.first()
    }

    /// Make `project` the only one, and keep the list
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn replace_all(&mut self, project: Project) -> io::Result<()> {
        let list = vec![project];
        // Kept in memory only once it is on disk, so a failed write changes nothing
        write(&self.file, &list)?;
        self.list = list;
        Ok(())
    }

    /// Keep `session` as the PM session of the project `id`, when it is not already
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn set_pm_session(&mut self, id: &str, session: SessionId) -> io::Result<()> {
        let Some(project) = self.list.iter_mut().find(|project| project.id == id) else {
            return Ok(());
        };
        if project.pm_session.as_ref() == Some(&session) {
            return Ok(());
        }
        project.pm_session = Some(session);
        write(&self.file, &self.list)
    }
}

/// Write `list` beside `file` and rename it into place, so a crash leaves the
/// old list or the new one
fn write(file: &Path, list: &[Project]) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }
    let json = serde_json::to_vec_pretty(list).map_err(io::Error::other)?;
    let partial = file.with_extension("json.partial");
    fs::write(&partial, json)?;
    fs::rename(&partial, file)
}

/// Asks the maintainer for a folder: the path they picked, `None` when they closed the dialog
///
/// The app's is the dialog plugin's folder picker, which blocks until the
/// dialog closes. A test's is a script.
pub type Picker = Box<dyn Fn() -> Result<Option<PathBuf>, String> + Send + Sync>;

/// What an [`Opener`] builds a PM session from
#[derive(Debug)]
pub struct Opening<'a> {
    /// The tree the session's harness starts in
    pub tree: &'a Arc<ProcessTree>,

    /// The app's data folder, which holds the installed bridges
    pub data_dir: &'a Path,

    /// The project the session is the PM of
    pub project: &'a Project,

    /// The folder for the harness's state: `<data_dir>/projects/<id>/pm/`
    pub state_dir: &'a Path,

    /// Where the person's harness logins are kept
    pub logins: &'a Store,
}

/// Builds the PM's session for a project
///
/// The app builds one on its built-in default; a test builds one on the
/// scripted agent. The text of an error is for the maintainer to read.
pub type Opener = Box<dyn Fn(&Opening<'_>) -> Result<Session, String> + Send + Sync>;

/// What Tauri manages: the process tree, the projects and the PM's session
pub struct AppState {
    data_dir: PathBuf,
    tree: Arc<ProcessTree>,
    logins: Store,
    projects: Arc<Mutex<Projects>>,
    /// The first project's PM, `None` until the first send. Held only for a
    /// look or a swap, never while the opener runs.
    session: Mutex<Option<Arc<Session>>>,
    /// Held while the PM is opened or the project replaced, so two sends open
    /// one session and a replaced project never keeps the old one
    opening: Mutex<()>,
    opener: Opener,
    picker: Picker,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("data_dir", &self.data_dir)
            .field("tree", &self.tree)
            .field("projects", &self.projects)
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Open the state in `data_dir`, with `opener` to build each PM session from
    /// `logins`, `picker` to ask for a folder, and the reaper at `reaper`
    ///
    /// Sweeps what the previous run's file lists before the new tree starts
    /// anything, then writes a run file of its own. The engine starts `reaper`
    /// as the run's first process, so no session can be built before it runs.
    /// `None` starts no reaper, which leaves a crash or a `SIGKILL` unguarded
    /// and is what a test asks for.
    ///
    /// # Errors
    ///
    /// The data folder or a file in it cannot be read or written, the previous
    /// run's file is not a run file, or the reaper cannot be started.
    pub fn open(
        data_dir: PathBuf,
        logins: Store,
        opener: Opener,
        picker: Picker,
        reaper: Option<&Path>,
    ) -> io::Result<Self> {
        fs::create_dir_all(&data_dir)?;
        let run_file = data_dir.join(RUN_FILE);
        sweep_leftovers(&run_file)?;
        let run = RunId::new()?;
        let tree = match reaper {
            Some(path) => ProcessTree::with_reaper(path, run, run_file).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("could not start the reaper at {}: {error}", path.display()),
                )
            })?,
            None => ProcessTree::new(run, run_file)?,
        };
        let tree = Arc::new(tree);
        let projects = Projects::load(data_dir.join(PROJECTS_FILE))?;
        Ok(Self {
            data_dir,
            tree,
            logins,
            projects: Arc::new(Mutex::new(projects)),
            session: Mutex::new(None),
            opening: Mutex::new(()),
            opener,
            picker,
        })
    }

    /// The app's data folder
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The tree every process of this run starts in
    #[must_use]
    pub fn tree(&self) -> Arc<ProcessTree> {
        Arc::clone(&self.tree)
    }

    /// Ask the maintainer for the project's folder and keep it
    ///
    /// Returns the folder, `None` when they closed the dialog, and then keeps nothing.
    ///
    /// # Errors
    ///
    /// The picker failed, or `projects.json` cannot be written.
    pub fn pick_project(&self) -> Result<Option<PathBuf>, String> {
        let Some(path) = (self.picker)()? else {
            return Ok(None);
        };
        self.set_project(path.clone())
            .map_err(|error| format!("could not keep the project: {error}"))?;
        Ok(Some(path))
    }

    /// The folder of the project the maintainer picked, `None` before they pick one
    #[must_use]
    pub fn project_path(&self) -> Option<PathBuf> {
        lock(&self.projects)
            .first()
            .map(|project| project.path.clone())
    }

    /// Make `path` the project, with a state folder of its own, and drop the PM
    /// of the one before
    ///
    /// # Errors
    ///
    /// The id cannot be made, or `projects.json` cannot be written.
    pub fn set_project(&self, path: PathBuf) -> io::Result<()> {
        let project = Project {
            id: new_id()?,
            path,
            pm_session: None,
        };
        let _opening = lock(&self.opening);
        lock(&self.projects).replace_all(project)?;
        *lock(&self.session) = None;
        Ok(())
    }

    /// The first project's PM session, built on the first call
    ///
    /// # Errors
    ///
    /// No project is picked yet, or the opener could not build the session. The
    /// text is for the maintainer to read.
    pub fn pm_session(&self) -> Result<(Arc<Session>, ProjectHandle), String> {
        let _opening = lock(&self.opening);
        let project = lock(&self.projects)
            .first()
            .cloned()
            .ok_or("Pick the project's folder first.")?;
        let handle = ProjectHandle {
            id: project.id.clone(),
            projects: Arc::clone(&self.projects),
        };
        if let Some(session) = self.held_session() {
            return Ok((session, handle));
        }
        let state_dir = self
            .data_dir
            .join(PROJECTS_FOLDER)
            .join(&project.id)
            .join(PM_FOLDER);
        let session = Arc::new((self.opener)(&Opening {
            tree: &self.tree,
            data_dir: &self.data_dir,
            project: &project,
            state_dir: &state_dir,
            logins: &self.logins,
        })?);
        *lock(&self.session) = Some(Arc::clone(&session));
        Ok((session, handle))
    }

    /// The PM session when one is open, `None` when none is
    ///
    /// Never builds one and never waits for one being built, so it is safe on
    /// the main thread.
    #[must_use]
    pub fn held_session(&self) -> Option<Arc<Session>> {
        lock(&self.session).as_ref().map(Arc::clone)
    }

    /// Stop every process of this run
    ///
    /// # Errors
    ///
    /// The first process that could not be stopped, after the others were tried.
    pub fn stop_all(&self) -> io::Result<()> {
        self.tree.stop_all(STOP_GRACE).map(|_| ())
    }
}

/// One project's entry in `projects.json`, for a session's thread to write to
#[derive(Debug, Clone)]
pub struct ProjectHandle {
    id: String,
    projects: Arc<Mutex<Projects>>,
}

impl ProjectHandle {
    /// Keep `session` as the project's PM session
    ///
    /// # Errors
    ///
    /// `projects.json` cannot be written.
    pub fn keep_session(&self, session: SessionId) -> io::Result<()> {
        lock(&self.projects).set_pm_session(&self.id, session)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sixteen hex digits from the system's random bytes
fn new_id() -> io::Result<String> {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for one test
    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("kelpie-state-{}", new_id().unwrap()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn projects_json_round_trips_a_list() {
        let dir = Dir::new();
        let file = dir.0.join("projects.json");
        let mut projects = Projects::load(file.clone()).unwrap();
        assert!(projects.list().is_empty(), "no file is no projects");

        let first = Project {
            id: "p1".to_owned(),
            path: PathBuf::from("/work/web"),
            pm_session: None,
        };
        projects.replace_all(first.clone()).unwrap();
        projects
            .set_pm_session("p1", SessionId::from("s-1"))
            .unwrap();

        let reloaded = Projects::load(file.clone()).unwrap();
        assert_eq!(
            reloaded.list(),
            [Project {
                pm_session: Some("s-1".into()),
                ..first
            }]
        );

        // The rail adds more later: a list of two reads back as it was written
        let two = r#"[
            {"id":"a","path":"/work/a","pm_session":null},
            {"id":"b","path":"/work/b","pm_session":"s-2"}
        ]"#;
        fs::write(&file, two).unwrap();
        let reloaded = Projects::load(file).unwrap();
        assert_eq!(
            reloaded
                .list()
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(reloaded.list()[1].pm_session, Some("s-2".into()));
    }

    #[test]
    fn a_list_that_cannot_be_written_is_not_kept_in_memory() {
        let dir = Dir::new();
        let file = dir.0.join("projects.json");
        let mut projects = Projects::load(file.clone()).unwrap();
        let old = Project {
            id: "old".to_owned(),
            path: PathBuf::from("/work/old"),
            pm_session: None,
        };
        projects.replace_all(old.clone()).unwrap();
        // A folder where the partial file goes, so the write fails
        fs::create_dir(file.with_extension("json.partial")).unwrap();

        let new = Project {
            id: "new".to_owned(),
            path: PathBuf::from("/work/new"),
            pm_session: None,
        };
        projects.replace_all(new).unwrap_err();

        assert_eq!(projects.list(), [old]);
    }

    #[test]
    fn a_project_that_cannot_be_kept_leaves_the_session_of_the_old_one() {
        let dir = Dir::new();
        let state = AppState::open(
            dir.0.clone(),
            Store::memory(),
            Box::new(|_| Err("never built".to_owned())),
            Box::new(|| Ok(None)),
            None,
        )
        .unwrap();
        state.set_project(PathBuf::from("/work/old")).unwrap();
        fs::create_dir(dir.0.join("projects.json.partial")).unwrap();

        state.set_project(PathBuf::from("/work/new")).unwrap_err();

        assert_eq!(state.project_path(), Some(PathBuf::from("/work/old")));
    }

    #[test]
    fn a_file_that_is_not_a_list_is_refused_naming_it() {
        let dir = Dir::new();
        let file = dir.0.join("projects.json");
        fs::write(&file, r#"{"id":"a"}"#).unwrap();
        let error = Projects::load(file).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("projects.json"), "{error}");
    }

    #[test]
    fn a_session_waits_for_a_project() {
        let dir = Dir::new();
        let state = AppState::open(
            dir.0.clone(),
            Store::memory(),
            Box::new(|_| Err("never built".to_owned())),
            Box::new(|| Ok(None)),
            None,
        )
        .unwrap();
        assert_eq!(state.project_path(), None);
        let error = state.pm_session().map(|_| ()).unwrap_err();
        assert!(error.contains("Pick"), "{error}");
        assert!(
            dir.0.join("run.json").exists(),
            "the tree wrote its run file"
        );
    }

    #[test]
    fn the_reaper_is_looked_for_beside_the_executable() {
        assert_eq!(reaper_path().file_name().unwrap(), REAPER);
        assert_eq!(
            reaper_path().parent(),
            std::env::current_exe().unwrap().parent(),
            "the app looks for its reaper beside its own executable"
        );
    }

    #[test]
    fn a_run_with_no_reaper_to_start_never_opens() {
        let dir = Dir::new();
        let missing = dir.0.join("not-a-reaper");
        let error = AppState::open(
            dir.0.clone(),
            Store::memory(),
            Box::new(|_| Err("never built".to_owned())),
            Box::new(|| Ok(None)),
            Some(&missing),
        )
        .unwrap_err();

        assert!(error.to_string().contains("not-a-reaper"), "{error}");
    }

    /// A stand-in reaper: it blocks on its standard input and exits when it ends
    ///
    /// The reaper binary is `kelpie-reaper`'s, and `scripts/build-reaper.sh`
    /// builds it into the sidecar folder. What these tests hold is the app's
    /// half: the tree it opens has a reaper running before a session can be built.
    #[cfg(unix)]
    fn stand_in_reaper(dir: &Dir) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;

        let path = dir.0.join("reaper");
        fs::write(&path, "#!/bin/sh\ncat >/dev/null\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Whether the process table still lists `pid` as running
    #[cfg(unix)]
    fn running(pid: u32) -> bool {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

        let pid = Pid::from_u32(pid);
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().without_tasks(),
        );
        system.process(pid).is_some_and(|process| {
            !matches!(
                process.status(),
                ProcessStatus::Zombie | ProcessStatus::Dead
            )
        })
    }

    #[test]
    #[cfg(unix)]
    fn the_reaper_starts_before_the_pm() {
        let dir = Dir::new();
        let reaper = stand_in_reaper(&dir);
        let state = AppState::open(
            dir.0.clone(),
            Store::memory(),
            Box::new(|opening| {
                let reaper = opening
                    .tree
                    .reaper_id()
                    .expect("the tree should hold a reaper by the time a PM is built");
                assert!(running(reaper), "the reaper runs while the PM is built");
                Err("no session here".to_owned())
            }),
            Box::new(|| Ok(None)),
            Some(&reaper),
        )
        .unwrap();

        let reaper = state
            .tree()
            .reaper_id()
            .expect("the run has a reaper of its own");
        assert!(running(reaper), "the reaper runs before any session exists");
        assert!(state.held_session().is_none(), "no session before a send");

        state.set_project(PathBuf::from("/work/one")).unwrap();
        // The opener runs on this thread, and asserts from inside the build
        state.pm_session().map(|_| ()).unwrap_err();

        // Dropping the state closes the reaper's pipe, and the stand-in exits on
        // the end of it. Waiting for that keeps the folder it lives in until the
        // shell has read it.
        drop(state);
        let deadline = std::time::Instant::now() + STOP_GRACE;
        while running(reaper) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !running(reaper),
            "the reaper exits when the tree is dropped"
        );
    }
}
