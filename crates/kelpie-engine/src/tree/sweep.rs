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

/// Stop what a run recorded, from its record already in hand
///
/// The same ladder as [`sweep_leftovers`], on a record read earlier: each
/// group whose leader still has its recorded start time gets `SIGTERM`, then
/// [`sweep_marked`] ends whatever still carries that run's mark, and last the
/// groups get `SIGKILL`, even when their leader has exited.
///
/// A caller that read the record once sweeps what it read. Re-reading the path
/// would sweep whatever the file holds by then, which a newer launch may own.
///
/// A group is stopped by signalling its leader's group, so only unix does that
/// step; the mark sweep runs everywhere.
///
/// # Errors
///
/// The record holds an id that is not a run id.
pub fn sweep_record(file: &RunFile) -> io::Result<SweepReport> {
    let run: RunId = file
        .run
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    #[cfg(unix)]
    let termed = term_groups(&file.groups);
    let report = sweep_marked(&run);
    #[cfg(unix)]
    kill_groups(&termed);
    Ok(report)
}

/// Stop what a previous run left behind, before anything of this run starts
///
/// Each group in the run file whose leader still has its recorded start time
/// gets `SIGTERM`. Then [`sweep_marked`] ends whatever still carries the old
/// run's mark, and last the groups get `SIGKILL`, even when their leader has
/// exited. A pid with another start time is a stranger and is left alone. A
/// missing file is a first launch.
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
    sweep_record(&file)
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

/// Send `SIGTERM` to each recorded group whose leader is still the recorded
/// process, wait for the leaders to exit, and return those groups' ids
#[cfg(unix)]
fn term_groups(groups: &[super::run_file::Group]) -> Vec<u32> {
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
    groups.iter().map(|group| group.pid).collect()
}

/// `SIGKILL` the groups [`term_groups`] returned
///
/// Their leaders matched a moment ago, and a group id is not reused while a
/// member lives, so a leader that has since exited does not make the group a
/// stranger's. This reaches a member that ignored `SIGTERM` and whose
/// environment the mark sweep cannot read.
#[cfg(unix)]
fn kill_groups(groups: &[u32]) {
    use shep_tree::Signal;
    use shep_tree::group::signal_group;

    for &pid in groups {
        let _ = signal_group(pid, Signal::Kill);
    }
}
