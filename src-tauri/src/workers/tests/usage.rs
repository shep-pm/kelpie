//! The ledger each project's worker turns write to, and what a quit writes

use std::fs;
use std::sync::Arc;

use kelpie_engine::login::Store;
use kelpie_engine::run::RunId;
use kelpie_engine::settings::ConfigDir;
use kelpie_engine::tree::ProcessTree;
use kelpie_engine::usage::{Draft, Ended, Line, Role, Timestamp};
use kelpie_engine::worker::Paths;

use super::Rig;
use crate::workers::Workers;

/// A turn on ticket `74`, started at the second `1_700_000_000`
fn draft() -> Draft {
    Draft {
        ticket: "74".to_owned(),
        role: Role::Worker,
        harness: None,
        model: None,
        effort: None,
        session: None,
        started: Timestamp::from_secs(1_700_000_000),
    }
}

/// The lines of `rig`'s project ledger
fn ledger(rig: &Rig) -> Vec<Line> {
    let kept = fs::read_to_string(Paths::new(&rig.dir, &rig.project).usage())
        .unwrap_or_else(|_| panic!("{} wrote no ledger", rig.project));
    kept.lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("a ledger line is not one JSON object"))
        .collect()
}

#[test]
fn a_turn_still_running_when_kelpie_quits_is_written_to_its_project_ledger_as_stopped() {
    let rig = Rig::new();
    let tree = Arc::new(ProcessTree::new(RunId::new().unwrap(), rig.dir.join("run.json")).unwrap());
    let workers = Workers::new(
        &rig.dir,
        &tree,
        &ConfigDir::at(rig.dir.join("config")),
        &Store::memory(),
    );
    let paths = Paths::new(&rig.dir, &rig.project);

    // The ledger a start meters into is the one held here, so the draft a start
    // opened is still reachable when kelpie quits
    workers.ledger(&paths, &rig.project).open(draft());
    assert!(
        !paths.usage().exists(),
        "a turn in flight has written nothing yet"
    );

    workers.stop_usage();

    let lines = ledger(&rig);
    let [line] = lines.as_slice() else {
        panic!("one stopped line for the turn kelpie cut off: {lines:#?}");
    };
    assert_eq!(line.ticket, "74");
    assert_eq!(line.role, Role::Worker);
    assert_eq!(line.ended, Ended::Stopped);

    workers.stop_usage();
    assert_eq!(ledger(&rig).len(), 1, "none twice");
}
