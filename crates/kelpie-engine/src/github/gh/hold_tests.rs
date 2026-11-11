//! A used-up limit holds every call, and the pace spaces the rest
//!
//! A scripted [`Runner`] stands in for `gh` and a fake clock for time, so no
//! test needs `gh`, a login or the network.
//!
//! [`Runner`]: super::Runner

use std::sync::{Arc, Mutex};

use super::hold::{CallPacer, FALLBACK, Hold, LocalPace, Pace, Pass, Wait};
use super::tests::{Script, answered, gh_over, refused};
use crate::github::{Clock, Timestamp};
use crate::ticket::ForgeError;

const NOW: u64 = 1_000_000;

// A clock that moves only when told to or when a call waits on it
#[derive(Debug, Clone)]
struct FakeClock {
    now: Arc<Mutex<u64>>,
    // Each wait, with how many calls the script had been asked by then
    waited: Arc<Mutex<Vec<(u64, usize)>>>,
    script: Script,
}

impl FakeClock {
    fn at(now: u64, script: &Script) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
            waited: Arc::default(),
            script: script.clone(),
        }
    }

    fn advance(&self, seconds: u64) {
        *self.now.lock().unwrap() += seconds;
    }

    fn waited(&self) -> Vec<(u64, usize)> {
        self.waited.lock().unwrap().clone()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_secs(*self.now.lock().unwrap())
    }
}

impl Wait for FakeClock {
    fn wait(&self, seconds: u64) {
        self.waited
            .lock()
            .unwrap()
            .push((seconds, self.script.asked().len()));
        self.advance(seconds);
    }
}

fn hold_over(script: &Script, clock: &FakeClock) -> Hold {
    Hold::with(
        gh_over(script),
        Box::new(LocalPace::new()),
        Arc::new(clock.clone()),
    )
}

fn at(seconds: u64) -> Timestamp {
    Timestamp::from_secs(NOW + seconds)
}

const USED_UP: &str = "gh: API rate limit exceeded for user ID 1. (HTTP 403)";

// What `gh api rate_limit` prints, with the core limit's `remaining` and `reset`
fn limits(remaining: u64, reset: u64) -> String {
    format!(
        r#"{{"resources":{{
            "core":{{"limit":5000,"used":{used},"remaining":{remaining},"reset":{reset}}},
            "graphql":{{"limit":5000,"used":0,"remaining":5000,"reset":{reset}}},
            "search":{{"limit":30,"used":0,"remaining":30,"reset":{reset}}}}}}}"#,
        used = 5000 - remaining,
    )
}

#[test]
fn a_used_up_limit_answers_held_with_its_reset_time_and_starts_no_child() {
    let script = Script::answering(refused(USED_UP));
    script.then(answered(&limits(0, NOW + 1_800)));
    let clock = FakeClock::at(NOW, &script);
    let hold = hold_over(&script, &clock);

    let first = hold.call(&["api", "repos/o/r"]);
    assert!(
        matches!(first, Err(ForgeError::Limited { .. })),
        "{first:?}"
    );
    let asked = script.asked().len();

    let second = hold.call(&["api", "repos/o/r"]);
    let third = hold.viewer();

    assert_eq!(second, Err(ForgeError::Held(at(1_800))));
    assert_eq!(third, Err(ForgeError::Held(at(1_800))));
    assert_eq!(hold.lifts_at(), Some(at(1_800)));
    assert_eq!(script.asked().len(), asked, "a held call starts no child");
}

#[test]
fn a_limit_with_no_reset_time_holds_for_ten_minutes() {
    let script = Script::answering(refused(USED_UP));
    // No limit is used up, so the read has no reset time to give
    script.then(answered(&limits(4_000, NOW + 1_800)));
    let clock = FakeClock::at(NOW, &script);
    let hold = hold_over(&script, &clock);

    hold.call(&["api", "user"]).unwrap_err();

    assert_eq!(FALLBACK, 600);
    assert_eq!(hold.call(&["api", "user"]), Err(ForgeError::Held(at(600))));
    assert_eq!(hold.lifts_at(), Some(at(600)));
}

#[test]
fn a_call_after_the_reset_time_goes_through() {
    let script = Script::answering(refused(USED_UP));
    script.then(answered(&limits(0, NOW + 1_800)));
    script.then(answered("octocat\n"));
    let clock = FakeClock::at(NOW, &script);
    let hold = hold_over(&script, &clock);
    hold.call(&["api", "user"]).unwrap_err();

    clock.advance(1_799);
    assert_eq!(hold.viewer(), Err(ForgeError::Held(at(1_800))));
    clock.advance(1);

    assert_eq!(hold.viewer(), Ok("octocat".to_owned()));
    assert_eq!(hold.lifts_at(), None);
}

#[test]
fn the_pace_stops_a_sixty_first_call_in_a_minute() {
    let script = Script::default();
    for _ in 0..61 {
        script.then(answered("{}"));
    }
    let clock = FakeClock::at(NOW, &script);
    let hold = hold_over(&script, &clock);

    // Five a second, so sixty calls take twelve seconds
    for second in 0..12 {
        for _ in 0..5 {
            hold.call(&["api", "user"]).unwrap();
        }
        assert_eq!(clock.waited(), [], "no wait at second {second}");
        clock.advance(1);
    }
    assert_eq!(script.asked().len(), 60);

    hold.call(&["api", "user"]).unwrap();

    // The first call left the minute 60 s after it went, 12 s ago
    assert_eq!(clock.waited(), [(48, 60)], "waited with 60 calls made");
    assert_eq!(script.asked().len(), 61);
}

#[test]
fn a_retry_after_holds_for_that_long_without_reading_the_limit() {
    let script = Script::answering(refused(
        "< HTTP/2.0 403 Forbidden\n< Retry-After: 90\n<\n\
         gh: You have exceeded a secondary rate limit. (HTTP 403)\n",
    ));
    let clock = FakeClock::at(NOW, &script);
    let hold = hold_over(&script, &clock);

    hold.call(&["api", "user"]).unwrap_err();

    assert_eq!(script.asked().len(), 1, "the reset was not asked for");
    assert_eq!(hold.lifts_at(), Some(at(90)));
}

#[test]
fn any_other_failure_holds_nothing() {
    let script = Script::answering(refused("gh: Not Found (HTTP 404)"));
    script.then(answered("octocat\n"));
    let clock = FakeClock::at(NOW, &script);
    let hold = hold_over(&script, &clock);

    assert_eq!(
        hold.call(&["api", "nowhere"]),
        Err(ForgeError::Failed("gh: Not Found (HTTP 404)".to_owned()))
    );

    assert_eq!(hold.lifts_at(), None);
    assert_eq!(hold.viewer(), Ok("octocat".to_owned()));
}

#[test]
fn a_hold_is_never_shortened_by_a_later_one() {
    let pace = LocalPace::new();
    pace.hold(at(100));
    pace.hold(at(50));

    assert_eq!(pace.pass(at(10)), Pass::Held(at(100)));
    assert_eq!(pace.held(at(99)), Some(at(100)));
    assert_eq!(pace.pass(at(100)), Pass::Go);
    assert_eq!(pace.held(at(100)), None);
}

#[test]
fn calls_past_the_minutes_cap_wait_until_the_oldest_leaves_the_minute() {
    let mut pacer = CallPacer::new(3, 10);
    assert_eq!(pacer.ask(at(0)), None);
    assert_eq!(pacer.ask(at(10)), None);
    assert_eq!(pacer.ask(at(20)), None);

    assert_eq!(pacer.ask(at(30)), Some(30));
    assert_eq!(pacer.ask(at(59)), Some(1));
    assert_eq!(pacer.ask(at(60)), None);
}

#[test]
fn calls_past_the_seconds_cap_wait_for_the_next_second() {
    let mut pacer = CallPacer::new(100, 2);
    assert_eq!(pacer.ask(at(0)), None);
    assert_eq!(pacer.ask(at(0)), None);
    assert_eq!(pacer.ask(at(0)), Some(1));
    assert_eq!(pacer.ask(at(1)), None);
}
