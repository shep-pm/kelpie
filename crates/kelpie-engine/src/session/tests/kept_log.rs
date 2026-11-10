//! Kelpie's own log of a session: what it keeps, and what reading it starts

use super::*;

/// A log file's path in a folder of this test's own
fn log_path() -> PathBuf {
    let dir = data_folder();
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("log.jsonl")
}

#[test]
fn the_log_round_trips_user_messages_and_events() {
    let agent = FakeAgent::new()
        .turn(Turn::new().text("Hel").text("lo"))
        .turn(Turn::new().text("Again"));
    let path = log_path();
    let session = session(&agent).with_log(log::Log::open(&path).unwrap());

    let first = say(&session, "Hi");
    let second = say(&session, "More");

    let mut expected = vec![LogEntry::User {
        text: "Hi".to_owned(),
    }];
    expected.extend(first.into_iter().map(LogEntry::Event));
    expected.push(LogEntry::User {
        text: "More".to_owned(),
    });
    expected.extend(second.into_iter().map(LogEntry::Event));
    assert_eq!(log::read(&path).unwrap(), expected);
}

#[test]
fn a_missing_log_is_an_empty_history() {
    let path = log_path();

    assert_eq!(log::read(&path).unwrap(), []);
}

#[test]
fn a_torn_last_line_is_dropped() {
    let path = log_path();
    let hi = LogEntry::User {
        text: "Hi".to_owned(),
    };
    let whole = serde_json::to_string(&hi).unwrap();
    // A crash cut the second line short, in the middle of a character
    let mut bytes = format!("{whole}\n{{\"entry\":\"user\",\"text\":\"h").into_bytes();
    bytes.push(0xE2);
    std::fs::write(&path, bytes).unwrap();

    assert_eq!(log::read(&path).unwrap(), [hi]);
}

#[test]
fn a_log_reopened_after_a_torn_write_carries_on_from_the_last_whole_line() {
    let path = log_path();
    let hi = LogEntry::User {
        text: "Hi".to_owned(),
    };
    std::fs::write(
        &path,
        format!("{}\n{{\"entry\":\"us", serde_json::to_string(&hi).unwrap()),
    )
    .unwrap();
    let more = LogEntry::User {
        text: "More".to_owned(),
    };

    log::Log::open(&path).unwrap().append(&more).unwrap();

    assert_eq!(log::read(&path).unwrap(), [hi, more]);
}

#[test]
fn a_line_that_is_not_the_last_and_not_an_entry_is_named() {
    let path = log_path();
    std::fs::write(&path, "nonsense\n{\"entry\":\"user\",\"text\":\"Hi\"}\n").unwrap();

    let error = log::read(&path).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("line 1"), "{error}");
}

#[test]
fn reading_history_starts_no_process() {
    let agent = FakeAgent::new().turn(Turn::new().text("Hello"));
    let path = log_path();
    let first = session(&agent).with_log(log::Log::open(&path).unwrap());
    say(&first, "Hi");
    drop(first);
    let launched = agent.launches();

    // A later launch: a session over the same log, and the history read before any send
    let relaunched = session(&agent).with_log(log::Log::open(&path).unwrap());
    let history = log::read(&path).unwrap();

    assert!(history.contains(&LogEntry::User {
        text: "Hi".to_owned()
    }));
    assert_eq!(agent.launches(), launched);
    drop(relaunched);
}

#[test]
fn a_closed_log_keeps_what_comes_after_out_of_it() {
    let agent = FakeAgent::new().turn(Turn::new().text("Working").until_cancelled());
    let path = log_path();
    let session = session(&agent).with_log(log::Log::open(&path).unwrap());
    let (sink, heard) = events();
    session.send("Go", sink);
    while !matches!(next(&heard), SessionEvent::Text { .. }) {}

    // The app is quitting: what the stop does to the session is not the maintainer's turn
    session.close_log();
    session.cancel();
    until_turn_ends(&heard);

    let history = log::read(&path).unwrap();
    assert!(
        history
            .iter()
            .all(|entry| !matches!(entry, LogEntry::Event(SessionEvent::TurnEnded { .. }))),
        "{history:?}"
    );
    assert!(history.contains(&LogEntry::User {
        text: "Go".to_owned()
    }));
}

#[test]
fn a_log_with_a_bad_line_in_the_middle_still_opens_and_appends() {
    let path = log_path();
    let hi = r#"{"entry":"user","text":"Hi"}"#;
    std::fs::write(&path, format!("nonsense\n{hi}\n")).unwrap();
    let more = LogEntry::User {
        text: "More".to_owned(),
    };

    log::Log::open(&path).unwrap().append(&more).unwrap();

    let held = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        held,
        format!(
            "nonsense\n{hi}\n{}\n",
            serde_json::to_string(&more).unwrap()
        )
    );
}
