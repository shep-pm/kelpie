//! The stand-in CI, which answers each head from a script and records each rerun

use super::fake::FakeCi;
use super::{Ci, FailedCheck, FailedLog, HeadStatus, Queued};
use crate::ticket::ForgeError;

#[test]
fn a_fake_answers_each_head_as_scripted_and_records_each_rerun() {
    let ci = FakeCi::new();
    let red = HeadStatus::Failed(vec![FailedCheck {
        name: "test".to_owned(),
        url: Some("https://github.com/shep-pm/kelpie/actions/runs/7/job/1".to_owned()),
    }]);
    let waiting = HeadStatus::Pending(vec![Queued {
        name: "lint".to_owned(),
        run: Some(8),
        running: false,
        held: false,
    }]);
    let log = FailedLog {
        check: "test".to_owned(),
        run: 7,
        url: "https://github.com/shep-pm/kelpie/actions/runs/7".to_owned(),
        tail: "error: boom".to_owned(),
    };
    ci.script_status("aaa", Ok(red.clone()));
    ci.script_status("bbb", Ok(waiting.clone()));
    ci.script_status("ccc", Err(ForgeError::Failed("HTTP 502".to_owned())));
    ci.script_logs("aaa", false, Ok(vec![log.clone()]));
    ci.script_rerun(9, Err(ForgeError::Failed("no such run".to_owned())));

    assert_eq!(ci.head_status("aaa"), Ok(red));
    assert_eq!(ci.head_status("bbb"), Ok(waiting));
    assert_eq!(
        ci.head_status("ccc"),
        Err(ForgeError::Failed("HTTP 502".to_owned()))
    );
    // A head nothing was scripted for has no check
    assert_eq!(ci.head_status("zzz"), Ok(HeadStatus::None));
    assert_eq!(ci.logs("aaa", false), Ok(vec![log]));
    // The queue's logs are not the head's
    assert_eq!(ci.logs("aaa", true), Ok(Vec::new()));
    assert_eq!(ci.rerun(8), Ok(()));
    assert_eq!(
        ci.rerun(9),
        Err(ForgeError::Failed("no such run".to_owned()))
    );
    assert_eq!(ci.reruns(), [8, 9]);
}
