//! Engine: the part of kelpie that starts, watches and stops every process, and holds the sessions
//!
//! ADR 0001 is the rule this crate is built to hold: every process a run
//! starts goes through the engine, each child marked with the run's id in its
//! environment. [`tree`] is the one way the engine starts a process, and the
//! sweep that stops what a run left behind. [`run`] is the id itself: what a
//! child's environment carries, and what a sweep looks for. [`settings`] reads
//! and writes the config folder ADR 0008 puts at `~/.config/kelpie`. [`bridge`]
//! installs the ACP registry bridges ADR 0007 runs harnesses through. [`session`]
//! holds a conversation with a harness over ACP (ADR 0007), and [`agents`] reads
//! the Markdown files that say what a session runs as and what it is told.
//! [`harness`] builds what one harness runs on: its bridge, the session's own
//! state folder, and the person's own login, which [`login`] keeps in the OS
//! keychain and never shows. [`sandbox`] puts each session's harness inside
//! crabpot, which confines what it reads, writes and reaches (ADR 0005). [`guard`]
//! reads a command a worker runs the way a shell would, and [`local_paths`] finds
//! what a text names of this machine. [`pacer`] holds a new turn while a limit's
//! window is spent, and [`worker`] caps how many of a project's workers run at
//! once. The crate has no Tauri dependency, so the app crate hosts it and the IPC
//! stays outside the engine (ADR 0002).
//!
//! # Quick start
//!
//! ```
//! use kelpie_engine::run::{ENV, RunId};
//!
//! let run = RunId::new()?;
//! assert_eq!(run.to_string().len(), 32);
//! assert_eq!(ENV, "KELPIE_RUN");
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! [`run`] holds the id and its text form; [`tree`] starts and stops the
//! children; [`session`] streams a harness's events; [`settings`] holds the
//! fields and the layers they resolve through; [`bridge`] installs a pinned
//! bridge once and checks it before it is used; [`agents`] loads the agent files
//! and the PM's binding; [`harness`] names the bridge and the login each harness
//! runs on; [`login`] holds those logins; [`sandbox`] builds the policy a
//! session runs under and starts its sandbox; [`guard`] splits a command, takes
//! off its wrappers and reads its scripts; [`local_paths`] is the one check for
//! what must not leave this machine; [`pacer`] holds a turn while a limit's window
//! is spent; [`worker`] counts how many of a project's workers may run at once.

#![doc(test(attr(deny(warnings))))]

pub mod agents;
pub mod bridge;
pub mod guard;
pub mod harness;
pub mod local_paths;
pub mod login;
pub mod pacer;
#[cfg(test)]
mod parity;
pub mod run;
pub mod sandbox;
pub mod session;
pub mod settings;
pub mod tree;
pub mod worker;
