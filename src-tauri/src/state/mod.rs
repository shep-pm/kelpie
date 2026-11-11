//! What the app holds between commands: the process tree, the projects and the PM's session
//!
//! [`AppState`] is what Tauri manages. It opens the run file in the app's data
//! folder, sweeping what the previous run left behind before anything starts
//! (ADR 0001), reads `projects.json` from the same folder, and hands the tree
//! the reaper beside the app's executable, which the engine starts as the run's
//! first process so a crash or a `SIGKILL` still stops what the run started.
//!
//! `projects.json` is a list from the start, each entry
//! `{ "id", "path", "pm_session", "pm_ran" }`, so the rail can add projects later without
//! a migration. Phase 1 has one: the folder the maintainer picks once. Each
//! project keeps its harness state in `<data_dir>/projects/<id>/pm/`.
//!
//! Each project's workers are kept in `<data_dir>/projects/<id>/workers.json`
//! ([`kelpie_engine::worker`]). A run that ended with a worker mid-turn is read
//! at the start of the next one, and every worker still holding a slot is marked
//! Paused: kelpie never restarts a worker on its own
//! (`docs/design/workers.md`, Quitting).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use kelpie_engine::bridge::{self, CheckOutcome, Pin, Versions};
use kelpie_engine::github::gh::GithubSource;
use kelpie_engine::harness::claude;
use kelpie_engine::login::Store;
use kelpie_engine::run::RunId;
use kelpie_engine::session::log::Log;
use kelpie_engine::session::{LogEntry, Session, SessionId};
use kelpie_engine::settings::{ConfigDir, Harness, Settings};
use kelpie_engine::tools::Tools;
use kelpie_engine::tree::{ProcessTree, sweep_leftovers};
use kelpie_engine::worker::{Paths, Records};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::pm::{self, PmPlan};
use crate::sandbox_setup::SandboxSetup;
use crate::workers::Workers;

/// The run file's name in the data folder
const RUN_FILE: &str = "run.json";

/// The list of projects, by name, in the data folder
const PROJECTS_FILE: &str = "projects.json";

/// The folder under the data folder that holds each project's state
const PROJECTS_FOLDER: &str = "projects";

/// The folder inside a project's folder that holds its PM's harness state
const PM_FOLDER: &str = "pm";

/// The PM's log, by name, in its state folder
const PM_LOG_FILE: &str = "log.jsonl";

/// The PM's standing-decisions file, by name, in its state folder
///
/// The PM's own instructions name it, and a new session is seeded from it.
const DECISIONS_FILE: &str = "decisions.md";

/// How long a quit gives each process to stop before the ladder goes on
const STOP_GRACE: Duration = Duration::from_secs(5);

/// How long kelpie waits between two looks for a newer version of each bridge
const BRIDGE_CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// The reaper's name, as `bundle.externalBin` in `tauri.conf.json` names it
///
/// Tauri reads that as `src-tauri/binaries/kelpie-reaper-sidecar-<target triple>`,
/// which `scripts/build-reaper.sh` writes, and copies it beside the app binary
/// under this name, the triple dropped. It is not `kelpie-reaper`, the name of
/// the cargo bin: `tauri-build` deletes whatever holds the name it copies to, so
/// the sidecar would replace the file cargo built.
const REAPER: &str = "kelpie-reaper-sidecar";

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

    /// The harness and model the PM last ran on, which says whose session
    /// `pm_session` is. `None` in a file from before this was kept.
    #[serde(default)]
    pub pm_ran: Option<Ran>,
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

    /// Keep `ran` as what the project `id`'s PM last ran on, when it is not already
    ///
    /// Another harness's session id cannot be resumed, so when the harness is
    /// not the one kept before, the saved session id is dropped in the same write.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn set_pm_ran(&mut self, id: &str, ran: &Ran) -> io::Result<()> {
        let Some(project) = self.list.iter_mut().find(|project| project.id == id) else {
            return Ok(());
        };
        if project.pm_ran.as_ref() == Some(ran) {
            return Ok(());
        }
        if project
            .pm_ran
            .as_ref()
            .is_some_and(|before| before.harness != ran.harness)
        {
            project.pm_session = None;
        }
        project.pm_ran = Some(ran.clone());
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

/// Mark each worker that was mid-turn when the last run ended as Paused
///
/// A launch never restarts a worker: `Paused` holds no slot and resumes only when
/// the maintainer says so (`GLOSSARY.md`, Paused), so each project's records
/// (`kelpie_engine::worker::Records`) are read at the start of the run and every
/// worker still holding a slot is marked Paused and gives it back. Its session id
/// is kept, so the maintainer can resume it.
///
/// One project's records file costs the launch nothing: it is left as it is and
/// named on stderr, so the PM and every other project still start. A file from a
/// newer kelpie, or one that cannot be read, is what a launch cannot settle.
fn pause_mid_turn_workers(data_dir: &Path, projects: &[Project]) {
    for project in projects {
        let file = Paths::new(data_dir, &project.id).records();
        let paused = Records::load(file.clone())
            .and_then(|mut records| records.pause_mid_turn())
            .map(|_| ());
        if let Err(error) = paused {
            eprintln!(
                "kelpie: {} was read without its workers paused: {error}",
                file.display()
            );
        }
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

    /// The folder holding every session's state folder: `<data_dir>/projects/`.
    /// A session reads its own state and no other session's.
    pub sessions_root: &'a Path,

    /// Where the person's harness logins are kept
    pub logins: &'a Store,

    /// The folder settings are read from, which says how each bridge is updated
    pub config: &'a ConfigDir,

    /// What the PM runs on, resolved from the settings when this send began
    pub pm: &'a PmPlan,

    /// kelpie's tools server, whose door each session's sandbox names and whose
    /// token each session's harness brings. `None` where the door could not open.
    pub tools: Option<&'a Tools>,

    /// Where the start records why its sandbox could not make one, and clears
    /// it when one can be made
    pub sandbox_setup: &'a SandboxSetup,
}

/// Builds the PM's session for a project
///
/// The app builds one on its built-in default; a test builds one on the
/// scripted agent. The text of an error is for the maintainer to read.
pub type Opener = Box<dyn Fn(&Opening<'_>) -> Result<Session, String> + Send + Sync>;

/// What the PM runs on: the harness and the model, kept to say what a switch left
// wire format: `projects.json` holds this as `pm_ran`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ran {
    /// The harness the PM runs on
    pub harness: Harness,

    /// The model it was asked for, `None` for the harness's own default
    pub model: Option<String>,
}

/// The PM session the app holds open, with what it was opened on
///
/// `None` until the first send. Held only for a look or a swap, never while the
/// opener runs.
type Held = Arc<Mutex<Option<(Ran, Arc<Session>)>>>;

/// What Tauri manages: the process tree, the projects and the PM's session
pub struct AppState {
    data_dir: PathBuf,
    tree: Arc<ProcessTree>,
    logins: Store,
    config: ConfigDir,
    projects: Arc<Mutex<Projects>>,
    /// The first project's PM and the harness it was opened on, `None` until
    /// the first send. Shared with the bridge checks, which prune what no
    /// session runs on.
    session: Held,
    /// Held while the PM is opened or the project replaced, so two sends open
    /// one session and a replaced project never keeps the old one
    opening: Mutex<()>,
    opener: Opener,
    picker: Picker,
    /// kelpie's tools server, with its door open for the whole run. `None` when
    /// the door could not open, which leaves the sessions without it.
    tools: Option<Tools>,
    /// What the PM's last start found about its sandbox
    sandbox_setup: SandboxSetup,
    /// Held so the bridge checks run; dropped on a quit, which wakes and ends them
    bridge_checks: Mutex<Option<Sender<()>>>,
    /// What the app keeps for its workers across starts
    workers: Workers,
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
    /// Open the state in `data_dir`, with settings and agent files read from
    /// `config`, `opener` to build each PM session from `logins`, `picker` to
    /// ask for a folder, and the reaper at `reaper`
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
        config: ConfigDir,
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
        pause_mid_turn_workers(&data_dir, projects.list());
        // A door that will not open costs the sessions kelpie's tools, not the app
        let tools = Tools::start(&data_dir)
            .map_err(|error| eprintln!("kelpie: the tools door did not open: {error}"))
            .ok();
        let workers = Workers::new(&data_dir, &tree, &config, &logins);
        Ok(Self {
            data_dir,
            tree,
            logins,
            config,
            projects: Arc::new(Mutex::new(projects)),
            session: Arc::new(Mutex::new(None)),
            opening: Mutex::new(()),
            opener,
            picker,
            tools,
            sandbox_setup: SandboxSetup::default(),
            bridge_checks: Mutex::new(None),
            workers,
        })
    }

    /// Look for a newer version of each installed bridge now, and again every 24 hours
    ///
    /// The checks run on a thread of their own, since each one waits on the
    /// network and on a probe. A bridge kelpie has not installed is skipped, and
    /// a quit ends them: [`AppState::stop_all`] wakes the thread, which then
    /// starts nothing more. Does nothing when they already run.
    pub fn start_bridge_checks(&self) {
        let mut held = lock(&self.bridge_checks);
        if held.is_some() {
            return;
        }
        let (stop, quit) = mpsc::channel();
        *held = Some(stop);
        let (data_dir, config, tree) = (self.data_dir.clone(), self.config.clone(), self.tree());
        let check = {
            let data_dir = data_dir.clone();
            move |pin: &Pin<'_>| {
                let policy = Settings::load_global(&config)
                    .map_err(|error| error.to_string())?
                    .bridge_updates_for(pin.id);
                Ok(bridge::check(
                    &data_dir,
                    &tree,
                    pin,
                    policy,
                    claude::REQUIRES,
                ))
            }
        };
        let prune = {
            let data_dir = data_dir.clone();
            let sessions = Arc::clone(&self.session);
            move |pin: &Pin<'_>| {
                // A version a session is still running on is never removed
                let running: Vec<Version> = lock(&sessions)
                    .iter()
                    .filter_map(|(_, session)| session.bridge_version())
                    .collect();
                let versions = Versions::load(&data_dir, pin).map_err(|error| error.to_string())?;
                versions
                    .prune(&data_dir, pin, &running)
                    .map_err(|error| error.to_string())
            }
        };
        thread::spawn(move || {
            run_bridge_checks(
                &[claude::pin()],
                &data_dir,
                BRIDGE_CHECK_EVERY,
                &quit,
                &check,
                &prune,
            );
        });
    }

    /// The app's data folder
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The folder settings and agent files are read from
    #[must_use]
    pub fn config(&self) -> &ConfigDir {
        &self.config
    }

    /// What the PM's last start found about its sandbox
    #[must_use]
    pub fn sandbox_setup(&self) -> &SandboxSetup {
        &self.sandbox_setup
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

    /// The folder of the project `id`, `None` when kelpie holds no such project
    #[must_use]
    pub fn project_path_of(&self, id: &str) -> Option<PathBuf> {
        lock(&self.projects)
            .list()
            .iter()
            .find(|project| project.id == id)
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
            pm_ran: None,
        };
        let _opening = lock(&self.opening);
        lock(&self.projects).replace_all(project)?;
        *lock(&self.session) = None;
        Ok(())
    }

    /// The first project's PM session, built on the first call
    ///
    /// What the PM runs on is resolved from the settings on every call, so a
    /// model or effort changed since the last message applies to this one, on
    /// the same session. A changed harness cannot move a session: the old
    /// session's process is stopped, a divider goes into the log, and a new
    /// session is built that has the seed ahead of its first message
    /// (`docs/design/pm-chat.md`, Switching harness or model). The divider's
    /// text is the third thing returned, `None` when nothing was switched.
    ///
    /// A new session that cannot be built leaves the old one running.
    ///
    /// # Errors
    ///
    /// No project is picked yet, the settings or the PM's agent file cannot be
    /// used, the opener could not build the session, or the divider could not
    /// be written. The text is for the maintainer to read.
    pub fn pm_session(&self) -> Result<(Arc<Session>, ProjectHandle, Option<String>), String> {
        let _opening = lock(&self.opening);
        // A refusal belongs to the start that found it: whatever fails before
        // the opener probes the sandbox again must not leave it on the chat.
        // A session already held keeps its own record, since it still runs as
        // that start left it, unsandboxed or not, and a send that reuses it
        // probes nothing.
        if lock(&self.session).is_none() {
            self.sandbox_setup.record(None);
        }
        let project = lock(&self.projects)
            .first()
            .cloned()
            .ok_or("Pick the project's folder first.")?;
        let handle = ProjectHandle {
            id: project.id.clone(),
            projects: Arc::clone(&self.projects),
        };
        let plan = pm::plan(&self.config, &project.id)?;
        let held = lock(&self.session).clone();
        let now = Ran {
            harness: plan.harness,
            model: plan.choice.model.clone(),
        };
        if let Some((was, session)) = &held
            && was.harness == plan.harness
        {
            session.choose(plan.choice.clone());
            if *was != now {
                self.keep_ran(&project.id, &now)?;
                if let Some((ran, _)) = lock(&self.session).as_mut() {
                    *ran = now;
                }
            }
            return Ok((Arc::clone(session), handle, None));
        }
        // What the PM ran on before this send: the open session, or what the
        // last launch kept. A session id belongs to the harness that gave it.
        let before = match &held {
            Some((was, _)) => Some(was.clone()),
            None => project.pm_ran.clone(),
        }
        .filter(|was| was.harness != plan.harness);
        let (state_dir, sessions_root) = (self.pm_dir(&project.id), self.sessions_root());
        let log = Log::open(state_dir.join(PM_LOG_FILE))
            .map_err(|error| format!("could not open the PM's log: {error}"))?;
        let decisions = state_dir.join(DECISIONS_FILE);
        let on = Project {
            pm_session: match before {
                Some(_) => None,
                None => project.pm_session.clone(),
            },
            ..project.clone()
        };
        let session = (self.opener)(&Opening {
            tree: &self.tree,
            data_dir: &self.data_dir,
            project: &on,
            state_dir: &state_dir,
            sessions_root: &sessions_root,
            logins: &self.logins,
            config: &self.config,
            pm: &plan,
            tools: self.tools.as_ref(),
            sandbox_setup: &self.sandbox_setup,
        })?
        .with_log(log.clone());
        // Kept before the old session stops, so a failed write leaves it running
        self.keep_ran(&project.id, &now)?;
        let mut divider = None;
        let session = match before {
            None => session.with_seed(decisions),
            Some(was) => {
                // The new session is built, so a failure above kept the old one
                if let Some((_, old)) = &held {
                    old.stop();
                }
                let text = format!(
                    "Switched from {} to {}",
                    pm::describe(was.harness, was.model.as_deref()),
                    pm::describe(now.harness, now.model.as_deref())
                );
                log.append(&LogEntry::Divider { text: text.clone() })
                    .map_err(|error| format!("could not write the PM's log: {error}"))?;
                divider = Some(text);
                session.with_seed_at_start(decisions)
            }
        };
        let session = Arc::new(session);
        *lock(&self.session) = Some((now, Arc::clone(&session)));
        Ok((session, handle, divider))
    }

    /// Keep `ran` as what the project `id`'s PM runs on
    fn keep_ran(&self, id: &str, ran: &Ran) -> Result<(), String> {
        lock(&self.projects)
            .set_pm_ran(id, ran)
            .map_err(|error| format!("could not write projects.json: {error}"))
    }

    /// The folder of the PM's harness state and log for the project `id`
    fn pm_dir(&self, id: &str) -> PathBuf {
        self.sessions_root().join(id).join(PM_FOLDER)
    }

    /// The folder holding every session's state folder, which no session reads
    /// but its own
    #[must_use]
    pub fn sessions_root(&self) -> PathBuf {
        self.data_dir.join(PROJECTS_FOLDER)
    }

    /// Where the first project's PM log is, `None` before a project is picked
    ///
    /// The path only: it opens nothing, so reading history never builds a
    /// session or starts a process.
    #[must_use]
    pub fn pm_log_path(&self) -> Option<PathBuf> {
        let id = lock(&self.projects).first()?.id.clone();
        Some(self.pm_dir(&id).join(PM_LOG_FILE))
    }

    /// The id of the project `id` names, or of the first when `None`
    ///
    /// # Errors
    ///
    /// No project is picked yet, or none has this id.
    pub fn project_id(&self, id: Option<&str>) -> Result<String, String> {
        let projects = lock(&self.projects);
        match id {
            Some(id) => projects
                .list()
                .iter()
                .find(|project| project.id == id)
                .map(|project| project.id.clone())
                .ok_or_else(|| format!("kelpie has no project {id}")),
            None => projects
                .first()
                .map(|project| project.id.clone())
                .ok_or_else(|| "Pick the project's folder first.".to_owned()),
        }
    }

    /// The PM session when one is open, `None` when none is
    ///
    /// Never builds one and never waits for one being built, so it is safe on
    /// the main thread.
    #[must_use]
    pub fn held_session(&self) -> Option<Arc<Session>> {
        lock(&self.session)
            .as_ref()
            .map(|(_, session)| Arc::clone(session))
    }

    /// What the app keeps for its workers across starts
    #[must_use]
    pub fn workers(&self) -> &Workers {
        &self.workers
    }

    /// kelpie's tools server, `None` when its door did not open
    #[must_use]
    pub fn tools(&self) -> Option<&Tools> {
        self.tools.as_ref()
    }

    /// The tickets of the project `id`, one source for the project, shared with the poller
    ///
    /// # Errors
    ///
    /// No such project, its origin is not a GitHub repo, or there is no `gh`.
    pub fn source(&self, id: &str) -> Result<Arc<GithubSource>, String> {
        let folder = self
            .project_path_of(id)
            .ok_or_else(|| format!("kelpie has no project {id}"))?;
        self.workers.source(id, &folder)
    }

    /// Stop every process of this run, and stop logging the PM and the workers
    ///
    /// # Errors
    ///
    /// The first process that could not be stopped, after the others were tried.
    pub fn stop_all(&self) -> io::Result<()> {
        // First, so the `Failed` that stopping the harness gives the turn in
        // flight stays out of the log, and the turn relaunches as interrupted
        if let Some(session) = self.held_session() {
            session.close_log();
        }
        // Before the tree stops, so no check starts a probe the stop would miss
        lock(&self.bridge_checks).take();
        self.workers.close_logs();
        self.tree.stop_all(STOP_GRACE).map(|_| ())
    }
}

/// Check each installed bridge in `pins`, prune what nothing needs, then wait
/// `every` and do it again
///
/// Ends when `quit`'s sender is dropped, whether it is waiting or about to check.
/// A failed check or prune is logged and the next one runs on schedule.
fn run_bridge_checks(
    pins: &[&Pin<'_>],
    data_dir: &Path,
    every: Duration,
    quit: &Receiver<()>,
    check: &dyn Fn(&Pin<'_>) -> Result<CheckOutcome, String>,
    prune: &dyn Fn(&Pin<'_>) -> Result<(), String>,
) {
    loop {
        for pin in pins {
            if matches!(quit.try_recv(), Err(mpsc::TryRecvError::Disconnected)) {
                return;
            }
            if !data_dir.join("bridges").join(pin.id).is_dir() {
                continue;
            }
            match check(pin) {
                Ok(CheckOutcome::Failed { reason }) | Err(reason) => {
                    eprintln!(
                        "kelpie: could not check {} for a newer version: {reason}",
                        pin.id
                    );
                }
                Ok(CheckOutcome::Unlisted { in_use }) => {
                    eprintln!(
                        "kelpie: {} not usable in the registry index, keeping {in_use}",
                        pin.id
                    );
                }
                Ok(CheckOutcome::HeldBack { version, reason }) => {
                    eprintln!("kelpie: {} {version} is held back: {reason}", pin.id);
                }
                Ok(_) => {}
            }
            if let Err(reason) = prune(pin) {
                eprintln!("kelpie: could not prune {} versions: {reason}", pin.id);
            }
        }
        if !matches!(quit.recv_timeout(every), Err(RecvTimeoutError::Timeout)) {
            return;
        }
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
mod tests;
