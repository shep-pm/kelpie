//! A real command inside crabpot, on macOS and Linux (ADR 0005)
//!
//! This file is its own `main`, as the app's is: crabpot runs the host binary
//! again as its helper on Linux, so [`run_from_args`] comes first. A test of
//! the same name in a `libtest` binary could not run there.
//!
//! Each topic is a file of its own under `tests/sandbox/`, named here with a
//! `#[path]` attribute, since a sandboxed test needs this binary's `main`. What
//! every topic needs is in `support` and `host`; `TESTS` below is the one list of
//! what runs.

#![cfg(unix)]
#![allow(
    missing_docs,
    reason = "a test binary with a main of its own, which no one links"
)]

use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use kelpie_engine::sandbox::run_from_args;
use kelpie_engine::tools;

// The scratch world every topic builds on, and the sandbox for it
#[path = "sandbox/support.rs"]
mod support;

// What the host can run: a sandbox that runs commands, or none at all
#[path = "sandbox/host.rs"]
mod host;

// Which tests an invocation names, read as `libtest` reads its arguments
#[path = "sandbox/selection.rs"]
mod selection;

// What a sandboxed command reads and writes
#[path = "sandbox/files.rs"]
mod files;

// The sweep finding a process kelpie started inside a sandbox
#[path = "sandbox/sweep.rs"]
mod sweep;

// kelpie's tools door, reached from a relay running inside a sandbox
#[path = "sandbox/relay.rs"]
mod relay;

// What a sandbox dials, and what a host reaches once request_network answers
#[path = "sandbox/network.rs"]
mod network;

// SK's sandbox escapes, tried against the PM's policy. Their own file, and run
// here, since a sandboxed test needs this binary's `main` on Linux
#[path = "sandbox/escapes.rs"]
mod escapes;

// The Harness global folder setting, tried against a real sandbox, in its own file
#[path = "sandbox/global_folder.rs"]
mod global_folder;

// A reviewer's sandbox: the worktree read only, its tests folder written, and no GitHub
#[path = "sandbox/reviewer.rs"]
mod reviewer;

/// How long a test waits for something it forced to happen
const WAIT: Duration = Duration::from_secs(20);

fn main() {
    run_from_args();
    // The relay is this binary run as `kelpie tools`, as the app's `main` runs it
    tools::run_from_args();
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some(sweep::SLEEP) => {
            sweep::sleep_in_the_sandbox(args.next().map(PathBuf::from));
            return;
        }
        Some(network::DIAL) => std::process::exit(network::dial(&args.next().unwrap_or_default())),
        // One of the escapes, in the copy of this binary its test started
        Some(escapes::IN_OWN_HOME) => escapes::run_case(&args.next().expect("a case's tag")),
        _ => {}
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|word| word == global_folder::IN_HOME)
    {
        let root = args
            .get(1)
            .expect("the scratch folder follows --in-the-persons-home");
        std::process::exit(global_folder::in_the_persons_home(Path::new(root)));
    }
    let selection = selection::Selection::of(&args);
    let mut failed = false;
    let (mut ran, mut filtered) = (0_usize, 0_usize);
    for (name, test) in TESTS {
        if !selection.runs(name) {
            filtered += 1;
            continue;
        }
        ran += 1;
        let skipped = host::SKIPPED.load(Ordering::SeqCst);
        match panic::catch_unwind(AssertUnwindSafe(test)) {
            Ok(()) if host::SKIPPED.load(Ordering::SeqCst) > skipped => {
                println!("test {name} ... SKIPPED");
            }
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed = true;
            }
        }
    }
    println!("ran {ran} tests, {filtered} filtered out");
    std::process::exit(i32::from(failed));
}

const TESTS: &[(&str, fn())] = &[
    (
        "the_value_of_a_libtest_option_is_not_a_filter",
        selection::the_value_of_a_libtest_option_is_not_a_filter,
    ),
    (
        "a_kernel_without_user_namespaces_is_told_from_a_command_that_failed",
        host::a_kernel_without_user_namespaces_is_told_from_a_command_that_failed,
    ),
    (
        "a_write_outside_the_state_dir_is_refused",
        files::a_write_outside_the_state_dir_is_refused,
    ),
    (
        "a_sandboxed_command_reads_the_project_and_writes_the_state_dir",
        files::a_sandboxed_command_reads_the_project_and_writes_the_state_dir,
    ),
    (
        "a_real_read_outside_the_project_follows_the_setting",
        files::a_real_read_outside_the_project_follows_the_setting,
    ),
    (
        "the_global_folder_is_read_only_when_on",
        global_folder::the_global_folder_is_read_only_when_on,
    ),
    (
        "the_sweep_finds_a_sandboxed_process",
        sweep::the_sweep_finds_a_sandboxed_process,
    ),
    (
        "the_relay_reaches_kelpie_from_inside_the_sandbox",
        relay::the_relay_reaches_kelpie_from_inside_the_sandbox,
    ),
    (
        "the_door_opens_nothing_else",
        relay::the_door_opens_nothing_else,
    ),
    (
        "an_allowed_host_is_reached_by_a_running_sandbox",
        network::an_allowed_host_is_reached_by_a_running_sandbox,
    ),
    (
        "escapes::harness_files_are_refused_in_any_case_and_spelling",
        escapes::harness_files_are_refused_in_any_case_and_spelling,
    ),
    (
        "escapes::a_nested_claude_folder_is_refused",
        escapes::a_nested_claude_folder_is_refused,
    ),
    (
        "escapes::the_pm_reads_the_project_and_no_other_repo_of_the_home",
        escapes::the_pm_reads_the_project_and_no_other_repo_of_the_home,
    ),
    (
        "escapes::a_403_to_the_connect_is_the_proxys_refusal_and_a_failed_connect_is_not",
        escapes::a_403_to_the_connect_is_the_proxys_refusal_and_a_failed_connect_is_not,
    ),
    (
        "escapes::a_host_not_allowed_and_a_socket_not_listed_are_refused",
        escapes::a_host_not_allowed_and_a_socket_not_listed_are_refused,
    ),
    (
        "a_denied_folder_under_an_allowed_one_stays_closed",
        files::a_denied_folder_under_an_allowed_one_stays_closed,
    ),
    (
        "reviewer::a_reviewer_cannot_write_the_worktree",
        reviewer::a_reviewer_cannot_write_the_worktree,
    ),
    (
        "reviewer::a_reviewer_writes_its_tests_folder",
        reviewer::a_reviewer_writes_its_tests_folder,
    ),
    (
        "reviewer::a_reviewer_cannot_reach_github",
        reviewer::a_reviewer_cannot_reach_github,
    ),
    // Only Linux turns a listed socket off; macOS reaches it
    #[cfg(target_os = "linux")]
    (
        "escapes::a_socket_the_policy_lists_is_still_refused_on_linux",
        escapes::a_socket_the_policy_lists_is_still_refused_on_linux,
    ),
];
