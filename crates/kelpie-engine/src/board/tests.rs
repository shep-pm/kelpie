//! The rule that puts a ticket in a column, its order, and the move table

use std::num::NonZeroU64;

use super::{Board, Column, Move, Read, Rules, place, plan_move};
use crate::github::Timestamp;
use crate::settings::PickUp;
use crate::ticket::{Listed, OpenPull, TicketId};
use crate::worker::{Record, Status};

const READY: &str = "ready-for-agent";

fn id(number: u64) -> TicketId {
    TicketId::from_issue(number).unwrap()
}

fn issue(number: u64, author: &str, labels: &[&str]) -> Listed {
    Listed {
        id: id(number),
        title: format!("ticket {number}"),
        labels: labels.iter().map(|label| (*label).to_owned()).collect(),
        author: author.to_owned(),
        created: Timestamp::from_secs(1_000 + number),
    }
}

fn pull(number: u64, branch: &str, closes: &[u64]) -> OpenPull {
    OpenPull {
        number: NonZeroU64::new(number).unwrap(),
        branch: branch.to_owned(),
        draft: true,
        closes: closes.iter().map(|closed| id(*closed)).collect(),
    }
}

fn worker(number: u64, status: Status) -> Record {
    let mut record = Record::new(number.to_string(), format!("kelpie/{number}"));
    record.status = status;
    record
}

fn me() -> Rules {
    Rules {
        pick_up: PickUp::Me,
        ready_label: READY.to_owned(),
        login: "rin".to_owned(),
    }
}

fn column_of(board: &Board, number: u64) -> Option<Column> {
    board
        .tickets()
        .iter()
        .find(|placed| placed.id == id(number))
        .map(|placed| placed.column)
}

fn ids(board: &Board, column: Column) -> Vec<u64> {
    board
        .column(column)
        .map(|placed| placed.id.github().unwrap())
        .collect()
}

#[test]
fn an_open_issue_without_the_ready_label_is_draft_and_one_with_it_is_ready() {
    let read = Read {
        ready: vec![issue(2, "rin", &[READY])],
        open: vec![issue(1, "rin", &[]), issue(2, "rin", &[READY])],
        ..Read::default()
    };

    let board = place(&read, &[], &me());

    assert_eq!(column_of(&board, 1), Some(Column::Draft));
    assert_eq!(column_of(&board, 2), Some(Column::Ready));
}

#[test]
fn a_live_worker_puts_a_ticket_in_building_whatever_its_labels_say() {
    for status in [Status::Running, Status::Paused] {
        let read = Read {
            ready: vec![issue(2, "rin", &[READY])],
            open: vec![
                issue(1, "rin", &[]),
                issue(2, "rin", &[READY]),
                issue(3, "rin", &[READY]),
            ],
            pulls: vec![pull(9, "other", &[3])],
            ..Read::default()
        };
        let workers = [worker(1, status), worker(2, status), worker(3, status)];

        let board = place(&read, &workers, &me());

        assert_eq!(ids(&board, Column::Building), [1, 2, 3], "{status:?}");
    }
}

/// A worker whose turn is running, or which is next to start, holds its ticket in
/// Building even when its pull request is open: a fix turn on red CI is such a
/// worker (`docs/design/gate-and-merge.md`, The Review column).
#[test]
fn a_running_worker_with_an_open_pull_is_in_building() {
    let read = Read {
        ready: vec![issue(3, "rin", &[READY])],
        open: vec![issue(3, "rin", &[READY])],
        pulls: vec![pull(9, "kelpie/3", &[3])],
        ..Read::default()
    };

    let board = place(&read, &[worker(3, Status::Running)], &me());

    assert_eq!(ids(&board, Column::Building), [3]);
    assert!(ids(&board, Column::Review).is_empty());
}

/// A worker that has asked the maintainer something runs nothing and holds no slot,
/// so its ticket stays where its open pull request puts it
#[test]
fn a_worker_waiting_on_the_maintainer_with_an_open_pull_is_in_review() {
    let read = Read {
        ready: vec![issue(2, "rin", &[READY]), issue(3, "rin", &[READY])],
        open: vec![
            issue(1, "rin", &[]),
            issue(2, "rin", &[READY]),
            issue(3, "rin", &[READY]),
            issue(4, "rin", &[READY]),
        ],
        pulls: vec![pull(9, "other", &[3]), pull(10, "kelpie/4", &[])],
        ..Read::default()
    };
    let workers = [
        worker(1, Status::WaitingOnMaintainer),
        worker(3, Status::WaitingOnMaintainer),
        worker(4, Status::WaitingOnMaintainer),
    ];

    let board = place(&read, &workers, &me());

    assert_eq!(ids(&board, Column::Review), [3, 4]);
    assert_eq!(ids(&board, Column::Building), [1]);
    assert_eq!(ids(&board, Column::Ready), [2]);
}

/// A waiting ticket with no pull request yet, such as one whose draft could not
/// open, is still the ticket its worker was started on
#[test]
fn a_worker_waiting_with_no_pull_is_in_building() {
    let read = Read {
        ready: vec![issue(2, "rin", &[READY])],
        open: vec![issue(1, "rin", &[]), issue(2, "rin", &[READY])],
        ..Read::default()
    };
    let workers = [
        worker(1, Status::WaitingOnMaintainer),
        worker(2, Status::WaitingOnMaintainer),
    ];

    let board = place(&read, &workers, &me());

    assert_eq!(ids(&board, Column::Building), [1, 2]);
    assert!(ids(&board, Column::Review).is_empty());
}

#[test]
fn an_ended_worker_leaves_the_ticket_where_its_issue_puts_it() {
    let read = Read {
        ready: vec![issue(2, "rin", &[READY])],
        open: vec![issue(1, "rin", &[]), issue(2, "rin", &[READY])],
        ..Read::default()
    };
    let workers = [worker(1, Status::Ended), worker(2, Status::Ended)];

    let board = place(&read, &workers, &me());

    assert_eq!(column_of(&board, 1), Some(Column::Draft));
    assert_eq!(column_of(&board, 2), Some(Column::Ready));
}

#[test]
fn an_open_pull_request_closing_a_ticket_puts_it_in_review() {
    let read = Read {
        ready: vec![issue(2, "rin", &[READY])],
        open: vec![issue(1, "rin", &[]), issue(2, "rin", &[READY])],
        pulls: vec![pull(40, "elsewhere", &[1, 2])],
        ..Read::default()
    };

    let board = place(&read, &[], &me());

    assert_eq!(ids(&board, Column::Review), [1, 2]);
    assert_eq!(board.tickets()[0].pull, NonZeroU64::new(40));
}

#[test]
fn a_pull_request_on_the_ticket_branch_puts_it_in_review() {
    let read = Read {
        open: vec![issue(1, "rin", &[]), issue(2, "rin", &[])],
        pulls: vec![pull(41, "kelpie/1", &[])],
        ..Read::default()
    };

    let board = place(&read, &[], &me());

    assert_eq!(column_of(&board, 1), Some(Column::Review));
    assert_eq!(column_of(&board, 2), Some(Column::Draft));
}

#[test]
fn a_closed_issue_is_done_even_with_a_pull_request_and_a_worker() {
    let read = Read {
        closed: vec![issue(1, "rin", &[READY])],
        pulls: vec![pull(42, "kelpie/1", &[1])],
        ..Read::default()
    };
    let workers = [worker(1, Status::Running)];

    let board = place(&read, &workers, &me());

    assert_eq!(ids(&board, Column::Done), [1]);
}

#[test]
fn me_takes_only_the_logins_issues_and_everyone_takes_all() {
    let read = Read {
        open: vec![
            issue(1, "rin", &[]),
            issue(2, "RIN", &[]),
            issue(3, "someone", &[]),
            issue(4, "", &[]),
        ],
        closed: vec![issue(5, "someone", &[]), issue(6, "Rin", &[])],
        ..Read::default()
    };

    let mine = place(&read, &[], &me());
    let everyone = place(
        &read,
        &[],
        &Rules {
            pick_up: PickUp::Everyone,
            ..me()
        },
    );

    assert_eq!(ids(&mine, Column::Draft), [1, 2]);
    assert_eq!(ids(&mine, Column::Done), [6]);
    assert_eq!(ids(&everyone, Column::Draft), [1, 2, 3, 4]);
    assert_eq!(ids(&everyone, Column::Done), [5, 6]);
}

#[test]
fn the_ready_label_makes_a_ticket_under_either_setting() {
    let read = Read {
        ready: vec![issue(1, "someone", &[READY])],
        open: vec![issue(1, "someone", &[READY]), issue(2, "someone", &[])],
        ..Read::default()
    };

    for pick_up in [PickUp::Me, PickUp::Everyone] {
        let board = place(&read, &[], &Rules { pick_up, ..me() });

        assert_eq!(column_of(&board, 1), Some(Column::Ready), "{pick_up:?}");
    }
    let board = place(&read, &[], &me());
    assert_eq!(column_of(&board, 2), None);
}

#[test]
fn an_entry_of_the_ready_list_is_a_ticket_whatever_the_case_of_its_label() {
    let read = Read {
        ready: vec![issue(1, "someone", &["ready-for-agent"])],
        ..Read::default()
    };
    let rules = Rules {
        ready_label: "Ready-For-Agent".to_owned(),
        ..me()
    };

    let board = place(&read, &[], &rules);

    assert_eq!(column_of(&board, 1), Some(Column::Ready));
}

#[test]
fn an_issue_in_both_lists_is_one_ticket_in_ready() {
    let mut in_ready = issue(1, "rin", &[READY]);
    in_ready.title = "from the ready list".to_owned();
    let mut in_open = issue(1, "rin", &[READY]);
    in_open.title = "from the open list".to_owned();
    let read = Read {
        ready: vec![in_ready],
        open: vec![in_open],
        ..Read::default()
    };

    let board = place(&read, &[], &me());

    assert_eq!(board.tickets().len(), 1);
    assert_eq!(board.tickets()[0].column, Column::Ready);
    assert_eq!(board.tickets()[0].title, "from the ready list");
}

#[test]
fn tickets_sort_by_priority_then_oldest_with_no_priority_last() {
    let mut old_none = issue(1, "rin", &[]);
    old_none.created = Timestamp::from_secs(10);
    let mut new_p0 = issue(2, "rin", &["priority: P0"]);
    new_p0.created = Timestamp::from_secs(900);
    let mut old_p2 = issue(3, "rin", &["priority: P2"]);
    old_p2.created = Timestamp::from_secs(20);
    let mut new_p2 = issue(4, "rin", &["priority: P2"]);
    new_p2.created = Timestamp::from_secs(80);
    let mut two = issue(5, "rin", &["priority: P3", "priority: P1", "bug"]);
    two.created = Timestamp::from_secs(500);
    let mut tied_b = issue(7, "rin", &["priority: P4"]);
    tied_b.created = Timestamp::from_secs(70);
    let mut tied_a = issue(6, "rin", &["priority: P4"]);
    tied_a.created = Timestamp::from_secs(70);
    let read = Read {
        open: vec![old_none, new_p0, old_p2, new_p2, two, tied_b, tied_a],
        ..Read::default()
    };

    let board = place(&read, &[], &me());

    assert_eq!(ids(&board, Column::Draft), [2, 5, 3, 4, 6, 7, 1]);
    assert_eq!(board.tickets()[1].priority, Some(1));
    assert_eq!(board.tickets()[6].priority, None);
}

#[test]
fn ready_to_building_is_start_and_the_same_column_is_stay() {
    assert_eq!(plan_move(Column::Ready, Column::Building), Move::Start);
    for column in COLUMNS {
        assert_eq!(plan_move(column, column), Move::Stay, "{column:?}");
    }
}

#[test]
fn every_other_move_snaps_back_with_a_line_naming_why() {
    let mut lines = std::collections::BTreeSet::new();
    for from in COLUMNS {
        for to in COLUMNS {
            if from == to || (from, to) == (Column::Ready, Column::Building) {
                continue;
            }
            let Move::SnapBack(line) = plan_move(from, to) else {
                panic!("{from:?} to {to:?} should snap back");
            };
            assert!(line.ends_with('.') && line.len() > 20, "{from:?} to {to:?}");
            lines.insert(line);
        }
    }

    let draft_to_ready = plan_move(Column::Draft, Column::Ready);
    let review_to_done = plan_move(Column::Review, Column::Done);
    let other = plan_move(Column::Done, Column::Draft);
    assert_eq!(lines.len(), 3);
    assert!(matches!(draft_to_ready, Move::SnapBack(line)
        if line.contains("ready label") && line.contains("Phase 3") && line.contains("GitHub")));
    assert!(matches!(review_to_done, Move::SnapBack(line)
        if line.contains("Merging") && line.contains("Phase 3")));
    assert!(matches!(other, Move::SnapBack(line)
        if line.contains("worker") && line.contains("issue")));
}

#[test]
fn a_column_is_written_in_lower_case() {
    let written: Vec<String> = COLUMNS
        .iter()
        .map(|column| serde_json::to_string(column).unwrap())
        .collect();

    assert_eq!(
        written,
        [
            r#""draft""#,
            r#""ready""#,
            r#""building""#,
            r#""review""#,
            r#""done""#
        ]
    );
    assert_eq!(
        serde_json::from_str::<Column>(r#""review""#).unwrap(),
        Column::Review
    );
}

const COLUMNS: [Column; 5] = [
    Column::Draft,
    Column::Ready,
    Column::Building,
    Column::Review,
    Column::Done,
];
