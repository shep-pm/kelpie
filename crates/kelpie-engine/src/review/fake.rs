//! A stand-in reviewer, for code that reads the port
//!
//! [`FakeReviewer`] answers each read from what a test scripted and records each
//! prompt, so a caller of [`Reviewer`] needs no session behind it.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{Finding, Read, Reviewer, ReviewerError};

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
