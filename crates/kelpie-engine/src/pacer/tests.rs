use std::sync::atomic::{AtomicU32, Ordering};

use super::*;

/// A fixed moment to read every window at, so no test depends on the clock
const EPOCH: u64 = 1_700_000_000;

/// Seconds in a day
const DAY_SECS: u64 = 86_400;

/// When the week resets: a week on from `EPOCH`
const RESETS: u64 = EPOCH + 7 * DAY_SECS;

/// The limit every test paces, named as a harness's adapter would name it
fn limit() -> LimitId {
    LimitId::new("codex")
}

/// A reading with `week` and `session` percent spent, the week resetting at [`RESETS`]
fn usage(week: u32, session: u32) -> Utilization {
    Utilization {
        session: Window {
            used_pct: session,
            resets_at: Timestamp::from_secs(EPOCH + 3 * 3600),
        },
        week: Window {
            used_pct: week,
            resets_at: Timestamp::from_secs(RESETS),
        },
    }
}

/// `hour` on the `day` after [`EPOCH`]
fn at(day: u64, hour: u64) -> Timestamp {
    Timestamp::from_secs(EPOCH + day * DAY_SECS + hour * 3600)
}

/// `secs` after `now`
fn after(now: Timestamp, secs: u64) -> Timestamp {
    Timestamp::from_secs(now.secs() + secs)
}

/// A meter that answers `usage`, counting how many times it was asked
struct Asked {
    usage: Utilization,
    times: AtomicU32,
}

impl Asked {
    fn new(usage: Utilization) -> Self {
        Self {
            usage,
            times: AtomicU32::new(0),
        }
    }

    fn asked(&self) -> u32 {
        self.times.load(Ordering::Relaxed)
    }
}

impl Meter for Asked {
    fn read(&self, _now: Timestamp) -> Result<Utilization, MeterError> {
        self.times.fetch_add(1, Ordering::Relaxed);
        Ok(self.usage)
    }
}

/// A meter that answers with what the harness said instead of usage
struct Refuses(&'static str);

impl Meter for Refuses {
    fn read(&self, _now: Timestamp) -> Result<Utilization, MeterError> {
        Err(MeterError::Unreadable(self.0.to_owned()))
    }
}

#[test]
fn the_window_holds_every_turn_only_once_spent() {
    for pct in [49, 50, 99] {
        let below = assess(&limit(), at(0, 1), &usage(0, pct));
        assert_eq!(below.hold, None, "{pct}% holds nothing");
    }

    let spent = assess(&limit(), at(0, 1), &usage(0, 100));
    let hold = spent.hold.expect("the window is spent");
    assert_eq!(hold.until, at(0, 3));
}

#[test]
fn a_spent_week_holds_every_turn_until_the_week_resets_and_outlasts_the_window() {
    let a = assess(&limit(), at(3, 1), &usage(100, 60));
    let hold = a.hold.expect("the week is spent");
    assert_eq!(hold.until, Timestamp::from_secs(RESETS));
    assert_eq!(
        hold.reason,
        "codex's week is at 100%, so no turn starts until it resets"
    );

    let almost = assess(&limit(), at(3, 1), &usage(99, 0));
    assert_eq!(almost.hold, None, "99% of the week holds nothing");
}

#[test]
fn a_spent_window_holds_every_turn_until_it_resets() {
    let now = at(0, 1);
    assert_eq!(
        spent(&limit(), now, &usage(0, 99)),
        None,
        "99% holds nothing"
    );

    let hold = spent(&limit(), now, &usage(0, 100)).expect("the window is spent");
    assert_eq!(hold.until, at(0, 3));
    assert_eq!(
        hold.reason,
        "codex's 5-hour window is at 100%, so no turn starts until it resets"
    );
}

#[test]
fn a_spent_week_and_a_spent_window_report_the_longer_hold() {
    let now = at(0, 1);
    let both = spent(&limit(), now, &usage(100, 100)).expect("both are spent");
    assert_eq!(
        both.until,
        Timestamp::from_secs(RESETS),
        "the week outlasts the window"
    );
    assert_eq!(
        both.reason,
        "codex's week is at 100%, so no turn starts until it resets"
    );

    let mut later = usage(100, 100);
    later.session.resets_at = Timestamp::from_secs(RESETS + 3600);
    let later = spent(&limit(), now, &later).expect("both are spent");
    assert_eq!(
        later.until,
        Timestamp::from_secs(RESETS + 3600),
        "the window outlasts the week"
    );
    assert_eq!(
        later.reason,
        "codex's 5-hour window is at 100%, so no turn starts until it resets"
    );
}

#[test]
fn a_window_that_has_not_started_holds_nothing() {
    // A harness prints `Current session: 0% used` as ending five hours on.
    let now = at(0, 1);
    let mut fresh = usage(0, 0);
    fresh.session.resets_at = after(now, 5 * 3600);
    assert_eq!(assess(&limit(), now, &fresh).hold, None);
}

#[test]
fn a_spent_window_whose_reset_has_already_come_holds_nothing() {
    // The reading is stale around a reset: it still says 100% an hour after the window reset.
    let now = at(0, 4);
    let stale = usage(0, 100);

    assert_eq!(
        assess(&limit(), now, &stale).hold,
        None,
        "a window that has reset holds nothing, even where the reading still says 100%"
    );

    let mut pacing = Pacing::default();
    let meter = Asked::new(stale);
    assert_eq!(
        pacing.pace(&limit(), Some(&meter), now),
        Pace::Clear,
        "no turn is held until a moment that has already gone by"
    );
    assert!(
        pacing.still_held(&limit(), now).is_none(),
        "a past-dated hold is not one a caller can wait out"
    );
}

#[test]
fn the_last_day_holds_nothing_until_the_week_is_spent() {
    let a = assess(&limit(), at(6, 3), &usage(88, 0));
    assert_eq!(a.hold, None);

    let spent = assess(&limit(), at(6, 20), &usage(100, 0));
    let hold = spent.hold.expect("the week is spent too");
    assert_eq!(
        hold.until,
        Timestamp::from_secs(RESETS),
        "a spent week holds every turn"
    );
}

#[test]
fn a_limit_id_with_no_meter_never_holds() {
    let mut pacing = Pacing::default();
    let local = LimitId::new("ollama");

    for round in 0..3u64 {
        assert_eq!(pacing.pace(&local, None, at(0, round)), Pace::Clear);
    }

    assert!(
        pacing.last.is_empty(),
        "a limit kelpie has no meter for is never read, so it holds nothing"
    );
    assert!(pacing.still_held(&local, at(0, 1)).is_none());
}

#[test]
fn a_spent_window_holds_the_next_turn_and_the_recheck_reads_it_again() {
    let mut pacing = Pacing::default();
    let meter = Asked::new(usage(0, 100));
    let now = at(0, 1);

    let first = pacing.pace(&limit(), Some(&meter), now);
    let Pace::Held(hold) = &first else {
        panic!("a spent window holds every turn: {first:?}");
    };
    assert_eq!(hold.until, at(0, 3));
    assert_eq!(meter.asked(), 1);

    let inside = pacing.pace(&limit(), Some(&meter), after(now, RECHECK_SECS / 2));
    assert_eq!(inside, first, "the hold is trusted until the recheck");
    assert_eq!(meter.asked(), 1, "no second read inside the recheck");

    let past = pacing.pace(&limit(), Some(&meter), after(now, RECHECK_SECS));
    assert_eq!(past, first, "the window is still spent");
    assert_eq!(meter.asked(), 2, "the recheck reads again");
}

#[test]
fn a_hold_is_never_trusted_past_its_own_reset() {
    let mut pacing = Pacing::default();
    let now = at(0, 1);
    let spent = Asked::new(Utilization {
        session: Window {
            used_pct: 100,
            resets_at: after(now, 300),
        },
        week: Window {
            used_pct: 0,
            resets_at: Timestamp::from_secs(RESETS),
        },
    });
    assert!(matches!(
        pacing.pace(&limit(), Some(&spent), now),
        Pace::Held(_)
    ));

    // The reset comes inside the recheck, and the window has cleared by it.
    let reset = Asked::new(usage(0, 0));
    assert_eq!(
        pacing.pace(&limit(), Some(&reset), after(now, 400)),
        Pace::Clear
    );
    assert_eq!(reset.asked(), 1, "the reset ends the hold, so it is read");
}

#[test]
fn a_meter_that_cannot_be_read_holds_nothing() {
    let mut pacing = Pacing::default();
    let meter = Refuses("claude printed no usage line");

    let pace = pacing.pace(&limit(), Some(&meter), at(0, 1));

    assert_eq!(
        pace,
        Pace::Unreadable("claude printed no usage line".to_owned())
    );
    assert!(
        pacing.still_held(&limit(), at(0, 1)).is_none(),
        "a reading kelpie does not have holds nothing"
    );
}

#[test]
fn each_limit_is_paced_on_its_own() {
    let mut pacing = Pacing::default();
    let spent = LimitId::new("claude-code");
    let clear = LimitId::new("codex");
    let spent_meter = Asked::new(usage(0, 100));
    let clear_meter = Asked::new(usage(0, 0));

    assert!(matches!(
        pacing.pace(&spent, Some(&spent_meter), at(0, 1)),
        Pace::Held(_)
    ));
    assert_eq!(
        pacing.pace(&clear, Some(&clear_meter), at(0, 1)),
        Pace::Clear,
        "the other limit's window is not spent"
    );
    assert!(pacing.still_held(&clear, at(0, 1)).is_none());
    assert!(pacing.still_held(&spent, at(0, 1)).is_some());
}
