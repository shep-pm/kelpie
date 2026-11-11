//! Where a pull request stands in the base branch's merge queue

use std::num::NonZeroU64;

use super::ForgeError;

/// Where a pull request stands in the merge queue
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueueStanding {
    /// Whether it is in the queue now
    pub queued: bool,
    /// Whether auto-merge is armed on it, to queue it once it can be
    pub armed: bool,
    /// How many times the queue has removed it, merged or not
    pub removals: u32,
    /// What the forge gave for the latest removal
    pub reason: Option<String>,
}

/// The base branch's merge queue
///
/// Where the base branch has one, the merge only queues the pull request and
/// then watches it (`docs/design/gate-and-merge.md`, The merge). The source
/// holds its repo, so no method names one.
pub trait MergeQueue: Send + Sync {
    /// Where pull request `number` stands in the queue now
    ///
    /// # Errors
    ///
    /// [`ForgeError`] when the forge cannot be asked or its answer read.
    fn standing(&self, number: NonZeroU64) -> Result<QueueStanding, ForgeError>;
}

#[cfg(any(test, feature = "testing"))]
pub use fake::FakeQueue;

#[cfg(any(test, feature = "testing"))]
mod fake {
    use std::collections::BTreeMap;
    use std::num::NonZeroU64;
    use std::sync::{Mutex, MutexGuard, PoisonError};

    use super::{ForgeError, MergeQueue, QueueStanding};

    type Answer = Result<QueueStanding, ForgeError>;

    #[derive(Debug, Default)]
    struct Script {
        answers: BTreeMap<NonZeroU64, Answer>,
        asked: Vec<NonZeroU64>,
    }

    /// A merge queue that answers each pull request from a script
    ///
    /// A pull request with no scripted answer is not in the queue, not armed
    /// and never removed.
    #[derive(Debug, Default)]
    pub struct FakeQueue {
        script: Mutex<Script>,
    }

    impl FakeQueue {
        /// A queue with nothing scripted
        #[must_use]
        pub fn new() -> Self {
            Self::default()
        }

        /// What [`MergeQueue::standing`] answers for pull request `number`
        pub fn script(&self, number: NonZeroU64, answer: Answer) {
            self.lock().answers.insert(number, answer);
        }

        /// The pull requests it was asked about, in order
        #[must_use]
        pub fn asked(&self) -> Vec<NonZeroU64> {
            self.lock().asked.clone()
        }

        fn lock(&self) -> MutexGuard<'_, Script> {
            self.script.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }

    impl MergeQueue for FakeQueue {
        fn standing(&self, number: NonZeroU64) -> Answer {
            let mut script = self.lock();
            script.asked.push(number);
            script
                .answers
                .get(&number)
                .cloned()
                .unwrap_or_else(|| Ok(QueueStanding::default()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(n: u64) -> NonZeroU64 {
        NonZeroU64::new(n).unwrap()
    }

    #[test]
    fn the_fake_answers_each_pull_as_scripted() {
        let queue = FakeQueue::new();
        let removed = QueueStanding {
            removals: 2,
            reason: Some("Required status check \"test\" failed.".into()),
            ..QueueStanding::default()
        };
        queue.script(
            number(7),
            Ok(QueueStanding {
                queued: true,
                ..QueueStanding::default()
            }),
        );
        queue.script(number(8), Ok(removed.clone()));
        queue.script(number(9), Err(ForgeError::Spawn("no gh".into())));

        assert!(queue.standing(number(7)).unwrap().queued);
        assert_eq!(queue.standing(number(8)).unwrap(), removed);
        assert_eq!(
            queue.standing(number(9)),
            Err(ForgeError::Spawn("no gh".into()))
        );
        assert_eq!(
            queue.standing(number(10)).unwrap(),
            QueueStanding::default()
        );
        assert_eq!(queue.asked(), [number(7), number(8), number(9), number(10)]);
    }
}
