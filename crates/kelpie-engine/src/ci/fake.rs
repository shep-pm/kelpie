//! A stand-in CI, for code that reads the port
//!
//! [`FakeCi`] answers each head from what a test scripted and records each
//! rerun, so a reader of [`Ci`] needs no CI behind it.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{Ci, FailedLog, HeadStatus};
use crate::ticket::ForgeError;

type Status = Result<HeadStatus, ForgeError>;
type Logs = Result<Vec<FailedLog>, ForgeError>;

#[derive(Debug, Default)]
struct Script {
    statuses: BTreeMap<String, Status>,
    logs: BTreeMap<(String, bool), Logs>,
    reruns: BTreeMap<u64, Result<(), ForgeError>>,
    rerun_calls: Vec<u64>,
}

/// A CI that answers from a script
///
/// A head with no scripted status has no check, a head with no scripted logs has
/// none to read, and a rerun succeeds unless one was scripted to fail.
#[derive(Debug, Default)]
pub struct FakeCi {
    script: Mutex<Script>,
}

impl FakeCi {
    /// A CI with nothing scripted
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What [`Ci::head_status`] answers for `head`
    pub fn script_status(&self, head: &str, answer: Status) {
        self.script().statuses.insert(head.to_owned(), answer);
    }

    /// What [`Ci::logs`] answers for `head`, with or without `merge_group`
    pub fn script_logs(&self, head: &str, merge_group: bool, answer: Logs) {
        self.script()
            .logs
            .insert((head.to_owned(), merge_group), answer);
    }

    /// What [`Ci::rerun`] answers for `run`
    pub fn script_rerun(&self, run: u64, answer: Result<(), ForgeError>) {
        self.script().reruns.insert(run, answer);
    }

    /// Each run it was asked to rerun, in order, whether or not the rerun failed
    #[must_use]
    pub fn reruns(&self) -> Vec<u64> {
        self.script().rerun_calls.clone()
    }

    fn script(&self) -> MutexGuard<'_, Script> {
        self.script.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Ci for FakeCi {
    fn head_status(&self, head: &str) -> Status {
        let script = self.script();
        script
            .statuses
            .get(head)
            .cloned()
            .unwrap_or(Ok(HeadStatus::None))
    }

    fn rerun(&self, run: u64) -> Result<(), ForgeError> {
        let mut script = self.script();
        script.rerun_calls.push(run);
        script.reruns.get(&run).cloned().unwrap_or(Ok(()))
    }

    fn logs(&self, head: &str, merge_group: bool) -> Logs {
        let script = self.script();
        script
            .logs
            .get(&(head.to_owned(), merge_group))
            .cloned()
            .unwrap_or_else(|| Ok(Vec::new()))
    }
}
