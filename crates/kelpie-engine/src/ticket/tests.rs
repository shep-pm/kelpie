//! The ticket id's text and serde, the error's retry and not-found calls, and
//! the fake that answers the port from a script

use serde_json::json;

use super::fake::{Asked, FakeTickets};
use super::{ForgeError, GithubTickets, Issue, Listed, TicketId, Tickets};
use crate::github::{Cooldown, Timestamp};

fn id(text: &str) -> TicketId {
    text.parse().unwrap()
}

#[test]
fn a_github_id_shows_as_its_number_and_a_local_one_as_k_and_its_counter() {
    for text in ["1", "340", "18446744073709551615", "k1", "k12"] {
        let ticket = id(text);
        assert_eq!(ticket.to_string(), text);
        assert_eq!(serde_json::to_value(ticket).unwrap(), json!(text));
    }
    assert!(matches!(id("340"), TicketId::Github(n) if n.get() == 340));
    assert!(matches!(id("k12"), TicketId::Local(n) if n.get() == 12));
    assert_eq!(id("340").github(), Some(340));
    assert_eq!(id("k12").github(), None);
}

#[test]
fn a_bare_number_reads_as_a_github_id() {
    for (value, want) in [
        (json!(340), "340"),
        (json!("340"), "340"),
        (json!("k12"), "k12"),
    ] {
        let read: TicketId = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(read, id(want), "{value}");
    }
}

#[test]
fn zero_is_no_ticket_id() {
    for text in ["0", "k0"] {
        assert!(text.parse::<TicketId>().is_err(), "{text}");
    }
    for value in [json!(0), json!("0"), json!("k0")] {
        assert!(
            serde_json::from_value::<TicketId>(value.clone()).is_err(),
            "{value}"
        );
    }
    assert_eq!(TicketId::from_issue(0), None);
    // A leading zero, a sign, a space and the wrong case are no id either.
    for text in [
        "",
        "#7",
        "007",
        "k012",
        "-7",
        "+7",
        " 7",
        "7 ",
        "K12",
        "kk1",
        "18446744073709551616",
    ] {
        assert!(text.parse::<TicketId>().is_err(), "{text:?}");
    }
}

#[test]
fn a_5xx_failure_passes_and_a_404_is_not_found() {
    // What the source's tool printed, and whether asking again may succeed
    for (said, passes, not_found) in [
        (
            "HTTP 502: 502 Bad Gateway (https://api.github.com/graphql)",
            true,
            false,
        ),
        ("gh: Bad Gateway (HTTP 502)", true, false),
        ("gh: Not Found (HTTP 404)", false, true),
        ("gh: Resource not accessible (HTTP 403)", false, false),
        ("no issue #7", false, false),
    ] {
        let failed = ForgeError::Failed(said.into());
        assert_eq!(failed.passes(), passes, "{said}");
        assert_eq!(failed.is_not_found(), not_found, "{said}");
    }
    // A held or limited read passes once the window is over; a refused ticket never does
    assert!(ForgeError::Held(Timestamp::from_secs(1)).passes());
    assert!(
        ForgeError::Limited {
            said: "slow down".into(),
            cooldown: Cooldown::default()
        }
        .passes()
    );
    assert!(!ForgeError::Unreadable("HTTP 502".into()).passes());
    assert!(!ForgeError::Spawn("not found".into()).passes());
    let none = ForgeError::NoIssue { ticket: id("k12") };
    assert!(!none.passes() && !none.is_not_found());
    assert_eq!(none.to_string(), "ticket k12 is not held by this source");
    assert!(!ForgeError::Unreadable("HTTP 404".into()).is_not_found());
}

fn listed(text: &str, title: &str) -> Listed {
    Listed {
        id: id(text),
        title: title.to_owned(),
        labels: vec!["ready-for-agent".to_owned()],
        author: "rin".to_owned(),
        created: Timestamp::from_secs(1_790_000_000),
    }
}

#[test]
fn a_fake_answers_each_port_method_from_its_script() {
    let issue = Issue {
        title: "A title".to_owned(),
        body: "A body".to_owned(),
        labels: vec!["bug".to_owned()],
        open: true,
    };
    let since = Timestamp::from_secs(1_789_000_000);
    let fake = FakeTickets::new();
    fake.script_issue(id("7"), Ok(issue.clone()));
    fake.script_issue(
        id("8"),
        Err(ForgeError::Failed("gh: Not Found (HTTP 404)".into())),
    );
    fake.script_ready(Ok(vec![listed("7", "Seven")]));
    fake.script_open(Ok(vec![listed("7", "Seven"), listed("9", "Nine")]));
    fake.script_closed(Err(ForgeError::Held(since)));

    // Asked through the traits, as a reader of the port does
    let tickets: &dyn Tickets = &fake;
    assert_eq!(tickets.issue(&id("7")), Ok(issue));
    assert!(tickets.issue(&id("8")).unwrap_err().is_not_found());
    assert_eq!(
        tickets.issue(&id("k1")),
        Err(ForgeError::NoIssue { ticket: id("k1") }),
        "a ticket the script has no answer for is one the source does not hold"
    );
    assert_eq!(
        tickets.ready_issues("ready-for-agent"),
        Ok(vec![listed("7", "Seven")])
    );
    let github: &dyn GithubTickets = &fake;
    assert_eq!(github.open_issues().unwrap().len(), 2);
    assert_eq!(github.closed_issues(since), Err(ForgeError::Held(since)));

    assert_eq!(
        fake.asked(),
        [
            Asked::Issue(id("7")),
            Asked::Issue(id("8")),
            Asked::Issue(id("k1")),
            Asked::Ready("ready-for-agent".to_owned()),
            Asked::Open,
            Asked::Closed(since),
        ]
    );
}
