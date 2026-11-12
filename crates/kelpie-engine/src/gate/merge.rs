//! What the maintainer's yes comes to, and the merge it sends
//!
//! A ready head merges on the maintainer's yes and on nothing else
//! (`docs/design/gate-and-merge.md`, The merge). The yes is kept with the head it
//! was given on ([`Kept::said_yes`]), and a pass that finds one decides here, from
//! plain values, whether it still holds. As SK's `merge()`, `withdraw`, `refused`
//! and `queued` (`runner/merge/mod.rs`, `queue.rs`), without `auto`, which comes
//! with the rulings: the caller reads the world, acts on the answer, and records
//! the act with the [`Kept`] methods once it is done.
//!
//! - [`yes`] says whether to merge, mark the draft ready, wait, or withdraw the yes.
//! - [`refused`] says what a merge the forge refused comes to.
//! - [`queue`] says what a merge sent to a merge queue has come to, and [`disarm`]
//!   whether an armed auto-merge is for a head that has gone bad.

use super::kept::{Kept, Sent};
use super::ruling::Ruling;
use super::{Base, CatchUp, Limits, Look, OpenPull, Timestamp, settled, short, waiting_since};
use crate::ci::HeadStatus;
use crate::ticket::QueueStanding;

/// What the maintainer's yes comes to on this pass
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Merge {
    /// The yes no longer holds, for this reason. The caller withdraws it and the order runs again.
    Withdraw(String),
    /// The pull request is a draft: mark it ready. The checks it starts are waited for.
    MarkReady,
    /// Nothing to do yet: the checks have not settled, or the forge's head is behind
    Wait,
    /// Merge the head the yes was given on, pinned to it
    Now,
}

/// Whether the maintainer's yes on `pull` still holds, and what to do about it
///
/// A yes holds only for the head it was given on, while CI on it is still green
/// and it still has the latest base. A marked draft starts a fresh run, so the
/// merge waits out [`CHECKS_SETTLE`](super::CHECKS_SETTLE) from the mark, which
/// the caller starts with [`Kept::restart_wait`]. Red CI withdraws the yes at
/// once, and a pending run is waited out, as the order does. Under Only on conflict, a branch behind its base that merges cleanly
/// still holds, as the order lets it through.
pub fn yes(
    look: &Look,
    pull: &OpenPull,
    kept: &mut Kept,
    limits: &Limits,
    now: Timestamp,
) -> Merge {
    if !kept.yes_holds(&pull.head) {
        return Merge::Withdraw(format!("its head moved to {}", short(&pull.head)));
    }
    let since = waiting_since(kept, &pull.head, now);
    let ci = limits.ci_holds_merge;
    let settling = ci && !settled(since, now);
    let green = match &pull.checks {
        _ if !ci => true,
        HeadStatus::Passed | HeadStatus::Skipped => true,
        // With no workflow on the base that runs on a pull request, no check will come
        HeadStatus::None if !look.base_runs_pull_workflow => true,
        // A run the ready mark started may outlast the settling time: the order waits on a
        // pending head too, so the yes does, and the order's own limits end a stuck run
        HeadStatus::Pending(_) => false,
        HeadStatus::None if settling => false,
        HeadStatus::None | HeadStatus::Failed(_) => {
            return Merge::Withdraw("CI is no longer green".to_owned());
        }
    };
    match look.base {
        Base::Current => {}
        Base::Lagging => return Merge::Wait,
        Base::Behind if limits.catch_up == CatchUp::Always || !look.merges_clean => {
            return Merge::Withdraw("the base moved since the yes".to_owned());
        }
        Base::Behind => {}
    }
    if pull.draft {
        return Merge::MarkReady;
    }
    if settling || !green {
        return Merge::Wait;
    }
    Merge::Now
}

/// What a merge the forge refused comes to
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The first refusal: the yes is withdrawn and the order runs again, which catches
    /// the branch up if the base moved under it
    Again,
    /// The second: the gate will not settle it itself
    Rule(Ruling),
}

/// What a merge of `head` the forge refused with `why` comes to
///
/// The first refusal goes back through the order once more, and a second, on any
/// head, is a ruling: merge refused. `kept` holds the refusals before this one, and
/// the caller records it with [`Kept::merge_refused`].
#[must_use]
pub fn refused(kept: &Kept, head: &str, why: &str) -> Refusal {
    if kept.merge_refusals == 0 {
        return Refusal::Again;
    }
    Refusal::Rule(Ruling::MergeRefused {
        head: head.to_owned(),
        why: why.to_owned(),
    })
}

/// What a merge sent to the merge queue has come to
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Queue {
    /// In the queue, or not yet seen there: nothing to do but wait
    Wait,
    /// Auto-merge is armed, to queue the pull request once it can be, and the head must still be the one
    Armed,
    /// The queue removed it, or it left the queue with no merge, for this reason
    Removed(String),
}

/// What `standing` says of a pull request whose merge was `sent`
///
/// A pull request open and in no queue, with no removal since kelpie sent it,
/// may only be slow to register, so it gets the checks' settling time before it
/// counts as gone.
#[must_use]
pub fn queue(sent: &Sent, standing: &QueueStanding, now: Timestamp) -> Queue {
    if standing.queued {
        return Queue::Wait;
    }
    if standing.armed {
        return Queue::Armed;
    }
    let removed = standing.removals > sent.removals;
    if !removed && !settled(sent.since, now) {
        return Queue::Wait;
    }
    Queue::Removed(match (removed, &standing.reason) {
        (true, Some(reason)) => reason.clone(),
        (true, None) => "it was taken out of the queue by hand".to_owned(),
        (false, _) => "it is neither merged nor in the queue".to_owned(),
    })
}

/// Why an armed auto-merge on `pull` is to be disarmed, none when it may stay
///
/// Auto-merge queues whatever head the branch has once it can, so one that waits
/// on a head the gate never saw, or on one gone red, is disarmed and goes back
/// through the order.
#[must_use]
pub fn disarm(sent: &Sent, pull: &OpenPull, limits: &Limits) -> Option<String> {
    if pull.head != sent.head {
        return Some(format!("its head moved to {}", short(&pull.head)));
    }
    if limits.ci_holds_merge && matches!(pull.checks, HeadStatus::Failed(_)) {
        return Some("CI is no longer green".to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::*;
    use crate::ci::FailedCheck;
    use crate::gate::{CHECKS_SETTLE, Pull};
    use crate::settings::CiWait;

    const HEAD: &str = "aaaaaaa1111111111111111111111111111111aa";
    const OTHER: &str = "bbbbbbb2222222222222222222222222222222bb";

    fn at(secs: u64) -> Timestamp {
        Timestamp::from_secs(secs)
    }

    fn limits() -> Limits {
        Limits {
            ci_holds_merge: true,
            fix_attempts: None,
            ci_wait: serde_json::from_value::<CiWait>(10.into()).unwrap(),
            catch_up: CatchUp::Always,
        }
    }

    fn pull(head: &str, checks: HeadStatus) -> OpenPull {
        OpenPull {
            number: NonZeroU64::new(71).unwrap(),
            branch: "kelpie/7".to_owned(),
            draft: false,
            closes: vec![],
            head: head.to_owned(),
            base: "main".to_owned(),
            labels: vec![],
            checks,
        }
    }

    fn look(pull: &OpenPull) -> Look {
        Look {
            pull: Pull::Open(pull.clone()),
            base: Base::Current,
            head: pull.head.clone(),
            agents_files: vec![],
            merges_clean: false,
            base_runs_pull_workflow: true,
        }
    }

    // A ticket the maintainer said yes on for `HEAD`, whose checks have had their time
    fn said_yes() -> Kept {
        let mut kept = Kept::default();
        kept.said_yes(HEAD);
        kept.wait = Some(crate::gate::Wait {
            head: HEAD.to_owned(),
            since: at(0),
        });
        kept
    }

    fn decide(pull: &OpenPull, base: Base, kept: &mut Kept, now: u64) -> Merge {
        let mut look = look(pull);
        look.base = base;
        yes(&look, pull, kept, &limits(), at(now))
    }

    #[test]
    fn a_yes_holds_only_for_the_head_it_was_given_on() {
        let green = pull(HEAD, HeadStatus::Passed);
        assert_eq!(
            decide(&green, Base::Current, &mut said_yes(), CHECKS_SETTLE),
            Merge::Now,
            "the head the yes was given on merges"
        );

        let moved = pull(OTHER, HeadStatus::Passed);
        assert_eq!(
            decide(&moved, Base::Current, &mut said_yes(), CHECKS_SETTLE),
            Merge::Withdraw("its head moved to bbbbbbb".to_owned()),
            "a head the yes was not given on does not"
        );
    }

    #[test]
    fn two_refusals_at_one_head_are_merge_refused() {
        let mut kept = said_yes();
        assert_eq!(
            refused(&kept, HEAD, "not mergeable"),
            Refusal::Again,
            "the first goes back through the order"
        );
        kept.merge_refused();

        assert_eq!(
            refused(&kept, HEAD, "not mergeable"),
            Refusal::Rule(Ruling::MergeRefused {
                head: HEAD.to_owned(),
                why: "not mergeable".to_owned(),
            }),
            "the second is a ruling"
        );
    }

    #[test]
    fn a_check_gone_red_withdraws_the_yes() {
        let pull = pull(
            HEAD,
            HeadStatus::Failed(vec![FailedCheck {
                name: "test".to_owned(),
                url: None,
            }]),
        );
        assert_eq!(
            decide(&pull, Base::Current, &mut said_yes(), CHECKS_SETTLE),
            Merge::Withdraw("CI is no longer green".to_owned())
        );
    }

    #[test]
    fn a_run_still_going_after_the_settling_time_is_waited_for() {
        let pending = pull(HEAD, HeadStatus::Pending(vec![]));
        assert_eq!(
            decide(&pending, Base::Current, &mut said_yes(), CHECKS_SETTLE * 10),
            Merge::Wait
        );
    }

    #[test]
    fn a_run_a_ready_mark_starts_is_waited_for_until_the_checks_settle() {
        let pending = pull(HEAD, HeadStatus::Pending(vec![]));
        assert_eq!(
            decide(&pending, Base::Current, &mut said_yes(), CHECKS_SETTLE - 1),
            Merge::Wait
        );
        let green = pull(HEAD, HeadStatus::Passed);
        assert_eq!(
            decide(&green, Base::Current, &mut said_yes(), CHECKS_SETTLE - 1),
            Merge::Wait,
            "a green rollup before the checks registered whole is not trusted"
        );
    }

    #[test]
    fn a_draft_is_marked_ready_first() {
        let mut draft = pull(HEAD, HeadStatus::Passed);
        draft.draft = true;
        assert_eq!(
            decide(&draft, Base::Current, &mut said_yes(), CHECKS_SETTLE),
            Merge::MarkReady
        );
    }

    #[test]
    fn a_moved_base_withdraws_the_yes_unless_only_a_conflict_catches_up_and_there_is_none() {
        let green = pull(HEAD, HeadStatus::Passed);
        assert_eq!(
            decide(&green, Base::Behind, &mut said_yes(), CHECKS_SETTLE),
            Merge::Withdraw("the base moved since the yes".to_owned())
        );
        let mut look = look(&green);
        look.base = Base::Behind;
        look.merges_clean = true;
        let mut only_conflicts = limits();
        only_conflicts.catch_up = CatchUp::Conflicts;
        assert_eq!(
            yes(
                &look,
                &green,
                &mut said_yes(),
                &only_conflicts,
                at(CHECKS_SETTLE)
            ),
            Merge::Now,
            "the order let a clean branch through unrebased, so the yes holds"
        );
    }

    #[test]
    fn a_forge_head_the_branch_has_not_caught_up_to_is_waited_on() {
        let green = pull(HEAD, HeadStatus::Passed);
        assert_eq!(
            decide(&green, Base::Lagging, &mut said_yes(), CHECKS_SETTLE),
            Merge::Wait
        );
    }

    #[test]
    fn a_project_that_does_not_wait_on_ci_merges_without_a_check() {
        let none = pull(HEAD, HeadStatus::None);
        let mut look = look(&none);
        let mut no_ci = limits();
        no_ci.ci_holds_merge = false;
        assert_eq!(
            yes(&look, &none, &mut said_yes(), &no_ci, at(0)),
            Merge::Now
        );
        look.base_runs_pull_workflow = false;
        assert_eq!(
            yes(&look, &none, &mut said_yes(), &limits(), at(CHECKS_SETTLE)),
            Merge::Now,
            "no workflow runs on a pull request, so no check will come"
        );
    }

    fn sent(removals: u32) -> Sent {
        let mut kept = Kept::default();
        kept.merge_sent(HEAD, removals, at(0));
        kept.sent.unwrap()
    }

    fn standing(queued: bool, armed: bool, removals: u32, reason: Option<&str>) -> QueueStanding {
        QueueStanding {
            queued,
            armed,
            removals,
            reason: reason.map(str::to_owned),
        }
    }

    #[test]
    fn a_queued_pull_is_waited_on_and_an_armed_one_is_looked_at() {
        let sent = sent(0);
        assert_eq!(
            queue(&sent, &standing(true, false, 0, None), at(10_000)),
            Queue::Wait
        );
        assert_eq!(
            queue(&sent, &standing(false, true, 0, None), at(10_000)),
            Queue::Armed
        );
    }

    #[test]
    fn a_removal_since_the_merge_was_sent_is_removed_with_the_forges_reason() {
        let sent = sent(1);
        assert_eq!(
            queue(&sent, &standing(false, false, 1, None), at(0)),
            Queue::Wait,
            "a removal from before the merge was sent is not this one"
        );
        assert_eq!(
            queue(
                &sent,
                &standing(false, false, 2, Some("Required status check failed.")),
                at(0)
            ),
            Queue::Removed("Required status check failed.".to_owned())
        );
        assert_eq!(
            queue(&sent, &standing(false, false, 2, None), at(0)),
            Queue::Removed("it was taken out of the queue by hand".to_owned())
        );
    }

    #[test]
    fn a_pull_in_no_queue_with_no_removal_is_gone_only_after_the_forge_had_time_to_show_it() {
        let sent = sent(0);
        let nowhere = standing(false, false, 0, None);
        assert_eq!(queue(&sent, &nowhere, at(CHECKS_SETTLE - 1)), Queue::Wait);
        assert_eq!(
            queue(&sent, &nowhere, at(CHECKS_SETTLE)),
            Queue::Removed("it is neither merged nor in the queue".to_owned())
        );
    }

    #[test]
    fn an_armed_auto_merge_is_disarmed_on_a_moved_head_or_a_red_one_only() {
        let sent = sent(0);
        let red = HeadStatus::Failed(vec![FailedCheck {
            name: "test".to_owned(),
            url: None,
        }]);
        assert_eq!(
            disarm(&sent, &pull(HEAD, HeadStatus::Passed), &limits()),
            None
        );
        assert_eq!(
            disarm(&sent, &pull(HEAD, red.clone()), &limits()),
            Some("CI is no longer green".to_owned())
        );
        assert_eq!(
            disarm(&sent, &pull(OTHER, HeadStatus::Passed), &limits()),
            Some("its head moved to bbbbbbb".to_owned())
        );
        let mut no_ci = limits();
        no_ci.ci_holds_merge = false;
        assert_eq!(disarm(&sent, &pull(HEAD, red), &no_ci), None);
    }
}
