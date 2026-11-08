//! Engine: the part of kelpie that starts, watches and stops every process, and holds the sessions
//!
//! Every process a run starts goes through here, each child marked with the
//! run's id in its environment (ADR 0001). It has no Tauri dependency, so the
//! app crate hosts it and the IPC stays outside the engine (ADR 0002).
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
//! [`run`] holds the id every child carries.

#![doc(test(attr(deny(warnings))))]

pub mod run;
