//! Tests for what the app holds

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
        pm_ran: None,
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
fn another_harness_drops_the_saved_session_in_the_same_write() {
    let dir = Dir::new();
    let file = dir.0.join("projects.json");
    let mut projects = Projects::load(file.clone()).unwrap();
    projects
        .replace_all(Project {
            id: "p1".to_owned(),
            path: PathBuf::from("/work/web"),
            pm_session: Some("s-1".into()),
            pm_ran: None,
        })
        .unwrap();
    let ran = |harness, model: &str| Ran {
        harness,
        model: Some(model.to_owned()),
    };

    // A file from before `pm_ran` was kept does not say whose id this is
    projects
        .set_pm_ran("p1", &ran(Harness::ClaudeCode, "opus"))
        .unwrap();
    projects
        .set_pm_ran("p1", &ran(Harness::ClaudeCode, "sonnet"))
        .unwrap();
    assert_eq!(projects.list()[0].pm_session, Some("s-1".into()));

    projects
        .set_pm_ran("p1", &ran(Harness::Goose, "qwen"))
        .unwrap();

    let reloaded = Projects::load(file).unwrap();
    assert_eq!(reloaded.list()[0].pm_session, None);
    assert_eq!(reloaded.list()[0].pm_ran, Some(ran(Harness::Goose, "qwen")));
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
        pm_ran: None,
    };
    projects.replace_all(old.clone()).unwrap();
    // A folder where the partial file goes, so the write fails
    fs::create_dir(file.with_extension("json.partial")).unwrap();

    let new = Project {
        id: "new".to_owned(),
        path: PathBuf::from("/work/new"),
        pm_session: None,
        pm_ran: None,
    };
    projects.replace_all(new).unwrap_err();

    assert_eq!(projects.list(), [old]);
}

#[test]
fn a_project_that_cannot_be_kept_leaves_the_session_of_the_old_one() {
    let dir = Dir::new();
    let state = AppState::open(
        dir.0.clone(),
        ConfigDir::at(dir.0.join("config")),
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
        ConfigDir::at(dir.0.join("config")),
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

/// Run the checks on a thread, counting what each one is asked, until `quit` drops
fn run_checks(
    dir: &Dir,
    every: Duration,
) -> (Sender<()>, thread::JoinHandle<()>, Arc<Mutex<Vec<String>>>) {
    let (stop, quit) = mpsc::channel();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let handle = {
        let (asked, data_dir) = (Arc::clone(&asked), dir.0.clone());
        thread::spawn(move || {
            run_bridge_checks(
                &[claude::pin()],
                &data_dir,
                every,
                &quit,
                &|pin| {
                    lock(&asked).push(pin.id.to_owned());
                    Ok(CheckOutcome::Current)
                },
                &|pin| {
                    let versions = Versions::load(&data_dir, pin).map_err(|e| e.to_string())?;
                    versions
                        .prune(&data_dir, pin, &[])
                        .map_err(|error| error.to_string())
                },
            );
        })
    };
    (stop, handle, asked)
}

/// Wait until `asked` holds `count` entries
fn wait_for(asked: &Mutex<Vec<String>>, count: usize) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while lock(asked).len() < count {
        assert!(std::time::Instant::now() < deadline, "the check never ran");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn an_installed_bridge_is_checked_at_once_and_again_every_interval() {
    let dir = Dir::new();
    fs::create_dir_all(dir.0.join("bridges").join(claude::pin().id)).unwrap();

    let (stop, handle, asked) = run_checks(&dir, Duration::from_millis(20));
    wait_for(&asked, 3);
    drop(stop);
    handle.join().unwrap();

    assert!(lock(&asked).iter().all(|id| id == claude::pin().id));
}

#[test]
fn a_bridge_never_installed_is_not_checked_and_a_quit_ends_the_wait() {
    let dir = Dir::new();

    let (stop, handle, asked) = run_checks(&dir, Duration::from_secs(24 * 60 * 60));
    drop(stop);
    handle.join().unwrap();

    assert!(lock(&asked).is_empty(), "{:?}", lock(&asked));
}

/// The version folders under the bridge `id`, oldest first
fn installed(dir: &Dir, id: &str) -> Vec<String> {
    let bridge = dir.0.join("bridges").join(id);
    let mut names: Vec<_> = fs::read_dir(&bridge)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_bridge_check_prunes_a_version_folder_nothing_needs() {
    let dir = Dir::new();
    let bridge = dir.0.join("bridges").join(claude::pin().id);
    fs::create_dir_all(bridge.join("0.100.0")).unwrap();
    fs::create_dir_all(bridge.join(claude::pin().version)).unwrap();

    let (stop, handle, asked) = run_checks(&dir, Duration::from_millis(20));
    wait_for(&asked, 1);
    drop(stop);
    handle.join().unwrap();

    assert_eq!(
        installed(&dir, claude::pin().id),
        [claude::pin().version],
        "kelpie's own pin is kept, and a version no session runs on is removed"
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
        ConfigDir::at(dir.0.join("config")),
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
        ConfigDir::at(dir.0.join("config")),
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
