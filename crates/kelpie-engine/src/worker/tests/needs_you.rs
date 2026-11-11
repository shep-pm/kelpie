//! What a record keeps of why its worker waits on the maintainer

use super::*;

/// Ticket `74` ended held: its branch changed a harness's own file
fn held(rig: &Rig, agent: &FakeAgent) -> Mid {
    let turn = mid_turn(rig, agent);
    commit(&turn.worktree, ".claude/settings.json", "{\"hooks\":{}}\n");
    turn.session.cancel();
    until_turn_ends(&turn.heard);
    turn
}

#[test]
fn a_held_turn_keeps_the_fences_line_as_the_records_reason() {
    let rig = Rig::new("reason kept");
    let agent = FakeAgent::new().turn(Turn::new().text("working").until_cancelled());

    held(&rig, &agent);

    let record = kept(&rig).remove(0);
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    assert_eq!(
        record.reason.as_deref(),
        Some("the branch changes a harness's own files: .claude/settings.json (Claude Code)"),
    );
}

#[test]
fn a_later_status_clears_the_reason() {
    let rig = Rig::new("reason cleared");
    let first = FakeAgent::new().turn(Turn::new().text("working").until_cancelled());
    held(&rig, &first);
    assert!(kept(&rig)[0].reason.is_some(), "the turn held the ticket");

    // The maintainer starts the ticket again, which makes it Running
    let binding = binding();
    let resumed = kept(&rig)[0].session.clone().expect("no session id kept");
    let second = FakeAgent::new()
        .can_resume()
        .knows(&resumed.to_string())
        .turn(Turn::new().text("again").until_cancelled());
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&second, opening)) };
    let (sink, heard) = events();
    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    assert!(matches!(started, Started::Running { .. }), "{started:?}");
    until_started(&heard);

    let record = kept(&rig).remove(0);
    assert_eq!(record.status, Status::Running);
    assert_eq!(record.reason, None, "a Running worker waits on nobody");
}

#[test]
fn an_old_workers_json_with_no_reason_still_loads() {
    let rig = Rig::new("reason old file");
    // A record as the launch before reasons wrote it
    std::fs::create_dir_all(rig.paths.records().parent().unwrap()).unwrap();
    std::fs::write(
        rig.paths.records(),
        r#"[{"ticket":"74","branch":"kelpie/74","session":null,"status":"waiting-on-maintainer","seat":"without"}]"#,
    )
    .unwrap();

    let records = Records::load(rig.paths.records()).unwrap();

    let record = records.get("74").expect("the old record loads");
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    assert_eq!(record.reason, None);
}
