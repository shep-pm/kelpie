//! Engine: the part of kelpie that starts, watches and stops every process, and holds the sessions
//!
//! ADR 0001 is the rule this crate is built to hold: every process a run
//! starts goes through the engine, each child marked with the run's id in its
//! environment. [`tree`] is the one way the engine starts a process, and the
//! sweep that stops what a run left behind. [`run`] is the id itself: what a
//! child's environment carries, and what a sweep looks for. [`settings`] reads
//! and writes the config folder ADR 0008 puts at `~/.config/kelpie`. [`session`]
//! holds a conversation with a harness over ACP (ADR 0007), and [`agents`] reads
//! the Markdown files that say what a session runs as and what it is told. The crate
//! has no Tauri dependency, so the app crate hosts it and the IPC stays outside
//! the engine (ADR 0002).
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
//! fields and the layers they resolve through; [`agents`] loads the agent files
//! and the PM's binding.

#![doc(test(attr(deny(warnings))))]

pub mod agents;
pub mod run;
pub mod session;
pub mod settings;
pub mod tree;
