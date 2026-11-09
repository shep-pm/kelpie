//! Engine: the part of kelpie that starts, watches and stops every process, and holds the sessions
//!
//! ADR 0001 is the rule this crate is built to hold: every process a run
//! starts goes through the engine, each child marked with the run's id in its
//! environment. [`tree`] is the one way the engine starts a process, and the
//! sweep that stops what a run left behind. [`run`] is the id itself: what a
//! child's environment carries, and what a sweep looks for. [`settings`] reads
//! and writes the config folder ADR 0008 puts at `~/.config/kelpie`. [`bridge`]
//! installs the ACP registry bridges ADR 0007 runs harnesses through. The crate
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
//! children; [`settings`] holds the fields and the layers they resolve through;
//! [`bridge`] installs a pinned bridge once and checks it before it is used.

#![doc(test(attr(deny(warnings))))]

pub mod bridge;
pub mod run;
pub mod settings;
pub mod tree;
