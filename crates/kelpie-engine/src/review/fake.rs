//! Stand-ins for the reviewer and the test runner, for code that reads the ports
//!
//! [`FakeReviewer`] answers each read from what a test scripted and records each
//! prompt, so a caller of [`Reviewer`] needs no session behind it.
//! [`FakeTestRunner`] answers each run of a reviewer's test from a script and
//! records what it was asked to run.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{Finding, Read, Reviewer, ReviewerError, TestOutcome, TestRun, TestRunner};

type Answer = Result<Vec<Finding>, ReviewerError>;

#[derive(Debug, Default)]
struct Script {
    answers: VecDeque<Answer>,
    prompts: Vec<String>,
}

/// A reviewer that answers from a script
///
/// Each read takes the next scripted answer. A read with none left finds
/// nothing, as a reviewer that says `CLEAN` does.
#[derive(Debug, Default)]
pub struct FakeReviewer {
    script: Mutex<Script>,
}

impl FakeReviewer {
    /// A reviewer with nothing scripted
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What the next read not yet answered answers
    pub fn script_read(&self, answer: Answer) {
        self.script().answers.push_back(answer);
    }

    /// The prompt of each read, in order, whether or not the read failed
    #[must_use]
    pub fn prompts(&self) -> Vec<String> {
        self.script().prompts.clone()
    }

    fn script(&self) -> MutexGuard<'_, Script> {
        self.script.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Reviewer for FakeReviewer {
    fn read(&self, read: &Read<'_>) -> Answer {
        let mut script = self.script();
        script.prompts.push(read.prompt.to_owned());
        script.answers.pop_front().unwrap_or_else(|| Ok(Vec::new()))
    }
}

/// One run the test runner was asked for
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// What it was asked to run
    pub run: TestRun,
    /// The watched paths the run's folder held as it ran
    pub held: Vec<PathBuf>,
}

#[derive(Debug, Default)]
struct Runs {
    outcomes: VecDeque<TestOutcome>,
    watched: Vec<PathBuf>,
    left: Vec<PathBuf>,
    ran: Vec<Ran>,
}

/// A test runner that answers from a script
///
/// Each run takes the next scripted outcome. A run with none left passes.
#[derive(Debug, Default)]
pub struct FakeTestRunner {
    runs: Mutex<Runs>,
}

impl FakeTestRunner {
    /// A runner with nothing scripted
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues the outcomes of the next runs, one each
    pub fn script(&self, outcomes: impl IntoIterator<Item = TestOutcome>) {
        self.runs().outcomes.extend(outcomes);
    }

    /// Notes, for every later run, whether its folder holds `path`
    pub fn watch(&self, path: impl Into<PathBuf>) {
        self.runs().watched.push(path.into());
    }

    /// Has every later run write a file at `path` in its folder, as a test that
    /// leaves something behind does
    pub fn leave(&self, path: impl Into<PathBuf>) {
        self.runs().left.push(path.into());
    }

    /// Every run asked for, oldest first
    #[must_use]
    pub fn ran(&self) -> Vec<Ran> {
        self.runs().ran.clone()
    }

    fn runs(&self) -> MutexGuard<'_, Runs> {
        self.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl TestRunner for FakeTestRunner {
    fn run(&self, run: &TestRun) -> TestOutcome {
        let mut runs = self.runs();
        let held = (runs.watched.iter())
            .filter(|path| run.cwd.join(path).exists())
            .cloned()
            .collect();
        for path in &runs.left {
            let at = run.cwd.join(path);
            if let Some(folder) = at.parent() {
                std::fs::create_dir_all(folder).expect("the folder a left file is in");
            }
            std::fs::write(at, "left behind\n").expect("the file a run leaves");
        }
        runs.ran.push(Ran {
            run: run.clone(),
            held,
        });
        runs.outcomes.pop_front().unwrap_or(TestOutcome::Passed)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::review::Severity;

    fn read(reviewer: &FakeReviewer, prompt: &str) -> Answer {
        reviewer.read(&Read {
            prompt,
            worktree: Path::new("worktree"),
            tests: None,
            sink: &|_| {},
        })
    }

    #[test]
    fn a_fake_reviewer_answers_each_read_as_scripted_and_records_its_prompt() {
        let reviewer = FakeReviewer::new();
        let finding = Finding::new(
            Severity::High,
            "src/lib.rs",
            3,
            "it panics",
            "a caller sees it",
        );
        reviewer.script_read(Ok(vec![finding.clone()]));
        reviewer.script_read(Err(ReviewerError::Stopped));

        assert_eq!(read(&reviewer, "first"), Ok(vec![finding]));
        assert_eq!(read(&reviewer, "second"), Err(ReviewerError::Stopped));
        assert_eq!(read(&reviewer, "third"), Ok(vec![]));
        assert_eq!(reviewer.prompts(), ["first", "second", "third"]);
    }
}
