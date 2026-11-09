//! What the app holds between commands: the process tree, the projects and the PM's session
//!
//! [`AppState`] is what Tauri manages. It opens the run file in the app's data
//! folder, sweeping what the previous run left behind before anything starts
//! (ADR 0001), and reads `projects.json` from the same folder.
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
        self.list = vec![project];
        self.save()
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
        self.save()
    }

    /// Write the list beside the file and rename it into place, so a crash
    /// leaves the old list or the new one
    fn save(&self) -> io::Result<()> {
        if let Some(folder) = self.file.parent() {
            fs::create_dir_all(folder)?;
        }
        let json = serde_json::to_vec_pretty(&self.list).map_err(io::Error::other)?;
        let partial = self.file.with_extension("json.partial");
        fs::write(&partial, json)?;
        fs::rename(&partial, &self.file)
    }
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
    projects: Arc<Mutex<Projects>>,
    /// The first project's PM, `None` until the first send
    session: Mutex<Option<Arc<Session>>>,
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
    /// Open the state in `data_dir`, with `opener` to build each PM session and
    /// `picker` to ask for a folder
    ///
    /// Sweeps what the previous run's file lists before the new tree starts
    /// anything, then writes a run file of its own. Starts no process and no
    /// reaper.
    ///
    /// # Errors
    ///
    /// The data folder or a file in it cannot be read or written, or the
    /// previous run's file is not a run file.
    pub fn open(data_dir: PathBuf, opener: Opener, picker: Picker) -> io::Result<Self> {
        fs::create_dir_all(&data_dir)?;
        let run_file = data_dir.join(RUN_FILE);
        sweep_leftovers(&run_file)?;
        let tree = Arc::new(ProcessTree::new(RunId::new()?, run_file)?);
        let projects = Projects::load(data_dir.join(PROJECTS_FILE))?;
        Ok(Self {
            data_dir,
            tree,
            projects: Arc::new(Mutex::new(projects)),
            session: Mutex::new(None),
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
        let mut session = lock(&self.session);
        lock(&self.projects).replace_all(project)?;
        *session = None;
        Ok(())
    }

    /// The first project's PM session, built on the first call
    ///
    /// # Errors
    ///
    /// No project is picked yet, or the opener could not build the session. The
    /// text is for the maintainer to read.
    pub fn pm_session(&self) -> Result<(Arc<Session>, ProjectHandle), String> {
        let mut held = lock(&self.session);
        let project = lock(&self.projects)
            .first()
            .cloned()
            .ok_or("Pick the project's folder first.")?;
        let handle = ProjectHandle {
            id: project.id.clone(),
            projects: Arc::clone(&self.projects),
        };
        if let Some(session) = held.as_ref() {
            return Ok((Arc::clone(session), handle));
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
        })?);
        *held = Some(Arc::clone(&session));
        Ok((session, handle))
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
            Box::new(|_| Err("never built".to_owned())),
            Box::new(|| Ok(None)),
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
}
