//! Finding and stopping what a run left running

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::time::Duration;

use shep_tree::ProcInstant;
use shep_tree::SweepReport;
use shep_tree::sweep::{LambSnapshot, ProcessTable, Sweeper};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

use super::run_file::RunFile;
use crate::run::{ENV, RunId};

/// How long a marked process has to exit on `SIGTERM` before it is killed
///
/// A CLI needs a moment to flush its session, and a launch can wait that long.
pub(super) const GRACE: Duration = Duration::from_secs(3);

/// How often a leftover group's leader is looked at during the grace
#[cfg(unix)]
const POLL: Duration = Duration::from_millis(50);

/// A process the table lists
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Reading {
    /// When it started, in seconds since the epoch
    pub started: u64,
    /// Whether it has exited and waits only to be reaped
    pub zombie: bool,
}

/// Look up one pid in the process table, `None` when no such process runs
pub(super) fn read(pid: u32) -> Option<Reading> {
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
    let process = system.process(pid)?;
    Some(Reading {
        started: process.start_time(),
        zombie: matches!(
            process.status(),
            ProcessStatus::Zombie | ProcessStatus::Dead
        ),
    })
}

/// Stop every process whose environment holds `KELPIE_RUN=<run>`, wherever it is
///
/// Signals `SIGTERM`, waits a few seconds, then kills what is left. A
/// process whose environment kelpie may not read is not found, such as a
/// system binary on macOS.
pub fn sweep_marked(run: &RunId) -> SweepReport {
    // Dated before the walk, so no pid it finds can be one that started after
    let seen_at = ProcInstant::now();
    let snapshot = LambSnapshot::new(marked(run), seen_at);
    Sweeper::new(snapshot, GRACE).run_blocking(&ProcessTable::new())
}

/// Stop what a previous run left behind, before anything of this run starts
///
/// Each group in the run file whose leader still has its recorded start time
/// gets `SIGTERM`, then `SIGKILL` after a few seconds; a pid with another start
/// time is a stranger and is left alone. Then [`sweep_marked`] ends whatever
/// still carries the old run's mark. A missing file is a first launch.
///
/// A group is stopped by signalling its leader's group, so only unix does that
/// step; the mark sweep runs everywhere.
///
/// # Errors
///
/// The run file cannot be read, is not a run file, or holds an id that is not
/// a run id.
pub fn sweep_leftovers(run_file: &Path) -> io::Result<SweepReport> {
    let file = match RunFile::read(run_file) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(SweepReport::default());
        }
        read => read?,
    };
    let run: RunId = file
        .run
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    #[cfg(unix)]
    stop_groups(&file.groups);
    Ok(sweep_marked(&run))
}

/// Pids of every other process marked with `run`
fn marked(run: &RunId) -> Vec<u32> {
    let mark = OsString::from(format!("{ENV}={run}"));
    let own = std::process::id();
    let mut system = System::new();
    // Threads would list again as processes, each with its process's environment
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .without_tasks()
            .with_environ(UpdateKind::Always),
    );
    system
        .processes()
        .values()
        .filter(|process| process.pid().as_u32() != own)
        .filter(|process| process.environ().contains(&mark))
        .map(|process| process.pid().as_u32())
        .collect()
}

#[cfg(unix)]
/// Whether `group`'s leader is still the process the run file recorded
fn recorded(group: &super::run_file::Group) -> Option<Reading> {
    read(group.pid).filter(|reading| reading.started == group.started)
}

#[cfg(unix)]
fn stop_groups(groups: &[super::run_file::Group]) {
    use shep_tree::Signal;
    use shep_tree::group::signal_group;
    use std::time::Instant;

    let own = std::process::id();
    let groups: Vec<&super::run_file::Group> = groups
        .iter()
        .filter(|group| group.pid != own && recorded(group).is_some())
        .collect();
    for group in &groups {
        let _ = signal_group(group.pid, Signal::Term);
    }
    let deadline = Instant::now() + GRACE;
    while Instant::now() < deadline
        && groups
            .iter()
            .any(|group| recorded(group).is_some_and(|leader| !leader.zombie))
    {
        std::thread::sleep(POLL);
    }
    // A zombie leader still holds its pid, so its group is still its own
    for group in groups.iter().filter(|group| recorded(group).is_some()) {
        let _ = signal_group(group.pid, Signal::Kill);
    }
}
