//! Starting a harness's program through the process tree, and saying how it ended
//!
//! [`ProcessLauncher`] is kelpie's own [`Launch`](super::Launch): it wraps the
//! command in the session's sandbox when it has one, hands the child's pipes to
//! ACP, and keeps the child so a session can say what the harness left behind
//! (ADR 0001). A launcher a [`Bridge`](super::Bridge) hands a session for one of
//! its versions ends the same way, through the same [`Ended`].

use std::io;
use std::path::PathBuf;
use std::process::{Command as Process, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use agent_client_protocol::DynConnectTo;

use crate::sandbox::Sandbox;
use crate::tree::{Child, ProcessTree};

use super::{Harness, Launch, Link, lock, stderr};

/// How long a harness that closed its pipes is given to show its exit status
const EXIT_GRACE: Duration = Duration::from_millis(500);

/// How long a stopped harness's stderr is given to be read to its end
const TAIL_SETTLE: Duration = Duration::from_millis(200);
const EXIT_POLL: Duration = Duration::from_millis(10);

/// Starts the harness's program through the process tree
///
/// kelpie builds every one itself, in [`launcher`](super::launcher),
/// [`sandboxed_launcher`](super::sandboxed_launcher) and the brief an
/// [`options`](super::options) ask runs on, so no launcher outside the engine
/// can be written.
pub(super) struct ProcessLauncher {
    pub(super) tree: Arc<ProcessTree>,
    pub(super) harness: Harness,
    /// Wraps the command before the tree spawns it
    pub(super) sandbox: Option<Sandbox>,
    pub(super) project: PathBuf,
}

impl Launch for ProcessLauncher {
    fn sandboxed(&self) -> bool {
        self.sandbox.is_some()
    }

    fn launch(&mut self) -> io::Result<Link> {
        let mut command = Process::new(&self.harness.program);
        command
            .args(&self.harness.args)
            .envs(self.harness.env.iter().map(|(name, value)| (name, value)))
            .current_dir(&self.project);
        // The wrapped command keeps the program, arguments, directory and
        // environment, and none of the streams
        let mut command = match &self.sandbox {
            Some(sandbox) => sandbox.command(&command)?,
            None => command,
        };
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = self.tree.spawn(&mut command)?;
        let (stdin, stdout, stderr) = (
            opened(child.stdin.take())?,
            opened(child.stdout.take())?,
            opened(child.stderr.take())?,
        );
        let tail = stderr::Tail::collect(stderr);
        let transport = agent_client_protocol::ByteStreams::new(
            blocking::Unblock::new(stdin),
            blocking::Unblock::new(stdout),
        );
        // Both closures reach the same child: a session only asks how it ended,
        // an ask only stops it
        let ended = Arc::new(Mutex::new(Ended { child, tail }));
        let shared = Arc::clone(&ended);
        Ok(Link {
            transport: DynConnectTo::new(transport),
            explain: Box::new(move || lock(&shared).explain()),
            stop: Box::new(move |grace| lock(&ended).stop(grace)),
        })
    }
}

/// The harness process kelpie started, and what it wrote to stderr
struct Ended {
    child: Child,
    tail: stderr::Tail,
}

impl Ended {
    /// How the harness ended, if it can be known
    fn explain(&mut self) -> Option<String> {
        explain_exit(&mut self.child, &self.tail)
    }

    /// Stop the harness's group, and say how it ended and what it last wrote
    fn stop(&mut self, grace: Duration) -> Option<String> {
        match self.child.stop(grace) {
            Ok(status) => {
                self.tail.settle(TAIL_SETTLE);
                Some(with_stderr(
                    format!("the harness exited with {status}"),
                    &self.tail,
                ))
            }
            Err(error) => Some(format!("kelpie could not stop the harness: {error}")),
        }
    }
}

fn opened<T>(pipe: Option<T>) -> io::Result<T> {
    pipe.ok_or_else(|| io::Error::other("a pipe was not opened"))
}

/// The exit status of `child` and the last of what it wrote to stderr
fn explain_exit(child: &mut Child, tail: &stderr::Tail) -> Option<String> {
    let deadline = Instant::now() + EXIT_GRACE;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(EXIT_POLL),
            Ok(None) | Err(_) => break None,
        }
    };
    let text = match status {
        Some(status) => format!("the harness exited with {status}"),
        None => "the harness closed its output but is still running".to_owned(),
    };
    Some(with_stderr(text, tail))
}

/// `text`, followed by the last of what the harness wrote to stderr, if it wrote any
fn with_stderr(mut text: String, tail: &stderr::Tail) -> String {
    let lines = tail.lines();
    if !lines.is_empty() {
        text.push_str("; it last wrote: ");
        text.push_str(&lines);
    }
    text
}
