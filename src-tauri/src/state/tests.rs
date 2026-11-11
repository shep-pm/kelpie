//! Tests for what the app holds

use std::sync::atomic::{AtomicUsize, Ordering};

use kelpie_engine::sandbox::Refusal;
use kelpie_engine::session::{Choice, Harness as SessionHarness};

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

#[test]
fn ending_the_pm_session_makes_the_next_send_open_a_new_one() {
    let dir = Dir::new();
    let opened = Arc::new(AtomicUsize::new(0));
    let state = AppState::open(
        dir.0.clone(),
        ConfigDir::at(dir.0.join("config")),
        Store::memory(),
        {
            let opened = Arc::clone(&opened);
            Box::new(move |opening| {
                opened.fetch_add(1, Ordering::SeqCst);
                // Found by this start, so the end of the session drops it
                opening
                    .sandbox_setup
                    .record_unsandboxed(Refusal::NoCapabilities);
                Ok(Session::new(
                    Arc::clone(opening.tree),
                    SessionHarness::new("kelpie-test-no-such-harness"),
                    opening.project.path.clone(),
                    None,
                    Choice::default(),
                ))
            })
        },
        Box::new(|| Ok(None)),
        None,
    )
    .unwrap();
    state.set_project(PathBuf::from("/work/one")).unwrap();

    let (first, ..) = state.pm_session().unwrap();
    let (again, ..) = state.pm_session().unwrap();
    assert!(
        Arc::ptr_eq(&first, &again),
        "a send reuses the held session"
    );
    assert_eq!(opened.load(Ordering::SeqCst), 1);
    assert!(state.sandbox_setup().view().unsandboxed);

    state.end_pm_session();
    assert!(state.held_session().is_none());
    assert!(!state.sandbox_setup().view().unsandboxed);

    let (next, ..) = state.pm_session().unwrap();
    assert_eq!(opened.load(Ordering::SeqCst), 2);
    assert!(
        !Arc::ptr_eq(&first, &next),
        "the next send has a new session"
    );

    // Ending a session that is not open is no error
    state.end_pm_session();
    state.end_pm_session();
}

#[test]
fn an_install_holds_the_app_until_it_ends_and_only_one_runs() {
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
    assert!(!state.installing());

    let hold = state.hold_for_install().expect("the first install holds");
    assert!(state.installing());
    assert!(
        state.hold_for_install().is_none(),
        "a second install would ask for a second password"
    );

    drop(hold);
    assert!(!state.installing());
    assert!(state.hold_for_install().is_some());
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

#[test]
fn a_launch_marks_every_worker_mid_turn_as_paused() {
    use kelpie_engine::worker::{Record, Seat, Status};

    let dir = Dir::new();
    let data = dir.0.join("data");
    let paths = Paths::new(&data, "p1");
    fs::create_dir_all(paths.project()).unwrap();
    fs::write(
        paths.records(),
        r#"[
            {"ticket":"74","branch":"kelpie/74","session":"s-1","status":"running","seat":"held"},
            {"ticket":"75","branch":"kelpie/75","session":null,"status":"running","seat":"waiting"},
            {"ticket":"76","branch":"kelpie/76","session":"s-3","status":"paused","seat":"without"}
        ]"#,
    )
    .unwrap();
    let projects = [Project {
        id: "p1".to_owned(),
        path: PathBuf::from("/work/web"),
        pm_session: None,
        pm_ran: None,
    }];

    pause_mid_turn_workers(&data, &projects);

    let kept = Records::load(paths.records()).unwrap();
    assert_eq!(
        kept.list(),
        [
            Record {
                ticket: "74".to_owned(),
                branch: "kelpie/74".to_owned(),
                session: Some("s-1".into()),
                status: Status::Paused,
                seat: Seat::Without,
                notes: Vec::new(),
                reason: None,
            },
            Record {
                ticket: "75".to_owned(),
                branch: "kelpie/75".to_owned(),
                session: None,
                status: Status::Running,
                seat: Seat::Waiting,
                notes: Vec::new(),
                reason: None,
            },
            Record {
                ticket: "76".to_owned(),
                branch: "kelpie/76".to_owned(),
                session: Some("s-3".into()),
                status: Status::Paused,
                seat: Seat::Without,
                notes: Vec::new(),
                reason: None,
            },
        ],
        "the worker mid-turn is Paused and holds no slot; the one waiting for a \
         slot keeps its place in line"
    );
    assert!(
        kept.get("74")
            .is_some_and(|record| record.session.is_some()),
        "the session id stays, so the maintainer can resume it"
    );
}

#[test]
fn a_launch_with_no_workers_keeps_nothing() {
    let dir = Dir::new();
    let data = dir.0.join("data");
    let projects = [Project {
        id: "p1".to_owned(),
        path: PathBuf::from("/work/web"),
        pm_session: None,
        pm_ran: None,
    }];

    pause_mid_turn_workers(&data, &projects);

    assert!(
        !Paths::new(&data, "p1").records().exists(),
        "no records file is written for a project that never started a worker"
    );
}

#[test]
fn one_projects_records_file_it_cannot_read_leaves_the_rest_paused() {
    use kelpie_engine::worker::Status;

    let dir = Dir::new();
    let data = dir.0.join("data");
    let broken = Paths::new(&data, "broken");
    let kept = Paths::new(&data, "kept");
    fs::create_dir_all(broken.project()).unwrap();
    fs::create_dir_all(kept.project()).unwrap();
    // A status this build does not know, which is what a downgrade leaves behind
    fs::write(
        broken.records(),
        r#"[{"ticket":"74","branch":"kelpie/74","session":null,"status":"queued","seat":"waiting"}]"#,
    )
    .unwrap();
    fs::write(
        kept.records(),
        r#"[{"ticket":"75","branch":"kelpie/75","session":"s-2","status":"running","seat":"held"}]"#,
    )
    .unwrap();
    let projects = [
        Project {
            id: "broken".to_owned(),
            path: PathBuf::from("/work/broken"),
            pm_session: None,
            pm_ran: None,
        },
        Project {
            id: "kept".to_owned(),
            path: PathBuf::from("/work/kept"),
            pm_session: None,
            pm_ran: None,
        },
    ];

    pause_mid_turn_workers(&data, &projects);

    assert_eq!(
        fs::read_to_string(broken.records()).unwrap(),
        r#"[{"ticket":"74","branch":"kelpie/74","session":null,"status":"queued","seat":"waiting"}]"#,
        "a file kelpie cannot read is left as it is"
    );
    assert_eq!(
        Records::load(kept.records()).unwrap().list()[0].status,
        Status::Paused,
        "the other project's worker is paused all the same"
    );
}
