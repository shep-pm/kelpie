//! kelpie-reaper: the process that stops the tree when kelpie dies without quitting
//!
//! The engine starts this binary and holds the write end of its standard input
//! open ([`kelpie_engine::tree::reaper`]). Reading ends when kelpie quits,
//! crashes or is killed, and this process then stops what the run file records
//! and whatever still carries the run's mark. It starts nothing, and it carries
//! no `KELPIE_RUN` of its own, so no sweep matches it.
//!
//! ```text
//! kelpie-reaper --run <run id> --run-file <path>
//! ```

use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read as _};
use std::path::PathBuf;
use std::process::ExitCode;

use kelpie_engine::run::RunId;
use kelpie_engine::tree::run_file::RunFile;
use kelpie_engine::tree::{sweep_marked, sweep_record};

fn main() -> ExitCode {
    let args = match Args::parse() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("kelpie-reaper: {error}");
            eprintln!("usage: kelpie-reaper --run <run id> --run-file <path>");
            return ExitCode::from(2);
        }
    };
    hold_the_pipe();
    match sweep(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("kelpie-reaper: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Read standard input to its end, discarding whatever arrives
///
/// The engine writes nothing to the pipe; the end is the event. A failed read
/// means no more bytes can arrive either, so it is waited out the same way.
fn hold_the_pipe() {
    let mut stdin = io::stdin().lock();
    let mut sink = [0u8; 64];
    loop {
        match stdin.read(&mut sink) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
    }
}

/// Stop what the run left behind
///
/// The record read here is the one swept, never the path read a second time: a
/// newer launch may have taken the file over by then, and its tree is not this
/// reaper's to stop. A record naming another run is that launch's to sweep, at
/// its own launch. With no record to read, the mark is all that is left, and it
/// names this reaper's run.
fn sweep(args: &Args) -> io::Result<()> {
    match RunFile::read(&args.run_file) {
        Ok(file) if file.run == args.run.to_string() => {
            sweep_record(&file)?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            sweep_marked(&args.run);
        }
        Err(error) => {
            eprintln!("kelpie-reaper: {error}");
            sweep_marked(&args.run);
        }
    }
    Ok(())
}

/// What the reaper was started with
#[derive(Debug)]
struct Args {
    /// The run whose tree this reaper stops
    run: RunId,
    /// Where that run records its groups
    run_file: PathBuf,
}

impl Args {
    /// Read the reaper's arguments from this process's command line
    ///
    /// # Errors
    ///
    /// See [`Usage`].
    fn parse() -> Result<Self, Usage> {
        Self::from_words(std::env::args_os().skip(1))
    }

    /// # Errors
    ///
    /// See [`Usage`].
    fn from_words(mut words: impl Iterator<Item = OsString>) -> Result<Self, Usage> {
        let mut run: Option<RunId> = None;
        let mut run_file: Option<PathBuf> = None;
        while let Some(flag) = words.next() {
            let name = match flag.to_str() {
                Some("--run") => "--run",
                Some("--run-file") => "--run-file",
                _ => return Err(Usage::Unknown(flag.to_string_lossy().into_owned())),
            };
            let Some(value) = words.next() else {
                return Err(Usage::MissingValue(name));
            };
            if name == "--run" {
                let text = value.to_string_lossy().into_owned();
                run = Some(text.parse().map_err(|_| Usage::NotRunId(text))?);
            } else {
                run_file = Some(PathBuf::from(value));
            }
        }
        Ok(Self {
            run: run.ok_or(Usage::Missing("--run"))?,
            run_file: run_file.ok_or(Usage::Missing("--run-file"))?,
        })
    }
}

/// Why a command line is not a reaper command line
#[derive(Debug, Clone, PartialEq, Eq)]
enum Usage {
    /// `--run` or `--run-file` is the last word, with no value after it
    MissingValue(&'static str),
    /// `--run` or `--run-file` never appears
    Missing(&'static str),
    /// `--run` is not 32 lowercase hex digits
    NotRunId(String),
    /// A word that is not `--run` or `--run-file`
    Unknown(String),
}

impl fmt::Display for Usage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingValue(name) => write!(f, "{name} has no value after it"),
            Self::Missing(name) => write!(f, "no {name}"),
            Self::NotRunId(text) => write!(f, "--run {text} is not 32 lowercase hex digits"),
            Self::Unknown(word) => write!(f, "unknown argument {word}"),
        }
    }
}

impl core::error::Error for Usage {}

#[cfg(test)]
mod tests {
    use super::*;

    const RUN: &str = "0123456789abcdef0123456789abcdef";

    fn parse(words: &[&str]) -> Result<Args, Usage> {
        Args::from_words(words.iter().map(OsString::from))
    }

    #[test]
    fn the_reaper_takes_a_run_and_a_run_file() {
        let args = parse(&["--run", RUN, "--run-file", "/tmp/kelpie/run.json"]).unwrap();
        assert_eq!(args.run.to_string(), RUN);
        assert_eq!(args.run_file, PathBuf::from("/tmp/kelpie/run.json"));
    }

    #[test]
    fn the_flags_may_come_in_any_order() {
        let args = parse(&["--run-file", "/tmp/kelpie/run.json", "--run", RUN]).unwrap();
        assert_eq!(args.run.to_string(), RUN);
        assert_eq!(args.run_file, PathBuf::from("/tmp/kelpie/run.json"));
    }

    #[test]
    fn a_flag_with_no_value_after_it_is_refused() {
        assert_eq!(
            parse(&["--run", RUN, "--run-file"]).unwrap_err(),
            Usage::MissingValue("--run-file")
        );
        assert_eq!(parse(&["--run"]).unwrap_err(), Usage::MissingValue("--run"));
    }

    #[test]
    fn a_missing_flag_is_refused() {
        assert_eq!(parse(&[]).unwrap_err(), Usage::Missing("--run"));
        assert_eq!(
            parse(&["--run", RUN]).unwrap_err(),
            Usage::Missing("--run-file")
        );
    }

    #[test]
    fn a_run_that_is_not_hex_is_refused() {
        assert_eq!(
            parse(&["--run", "not-a-run-id", "--run-file", "/tmp/run.json"]).unwrap_err(),
            Usage::NotRunId("not-a-run-id".into())
        );
    }

    #[test]
    fn a_word_that_is_not_a_flag_is_refused() {
        assert_eq!(
            parse(&["--reaper", RUN]).unwrap_err(),
            Usage::Unknown("--reaper".into())
        );
    }

    #[test]
    fn every_usage_names_what_is_wrong() {
        let cases = [
            (Usage::MissingValue("--run"), "--run"),
            (Usage::Missing("--run-file"), "--run-file"),
            (Usage::NotRunId("zz".into()), "zz"),
            (Usage::Unknown("--kill".into()), "--kill"),
        ];
        for (usage, named) in cases {
            let text = usage.to_string();
            assert!(text.contains(named), "{usage:?} => {text}");
        }
    }
}
