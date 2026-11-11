//! Tests for opening a ticket's page
//!
//! No test starts a browser or reaches a portal: the OS's broker is a stand-in
//! command, or a [`Run`] that records what it was asked to start and answers as
//! a test says.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};
use std::sync::{Arc, Mutex};

use kelpie_engine::github::ForgeSlug;
use kelpie_engine::run::RunId;
use kelpie_engine::tree::ProcessTree;

use crate::board::link::{LinkView, Os, Programs, Run, check, open_with};

fn repo() -> ForgeSlug {
    ForgeSlug::try_from("shep-pm/kelpie".to_owned()).unwrap()
}

#[cfg(unix)]
fn status(code: i32) -> ExitStatus {
    std::os::unix::process::ExitStatusExt::from_raw(code << 8)
}

#[cfg(windows)]
fn status(code: i32) -> ExitStatus {
    std::os::windows::process::ExitStatusExt::from_raw(code.cast_unsigned())
}

/// A program and the arguments it was started with
type Started = (PathBuf, Vec<String>);

/// How a [`Recorder`] answers each start
type Answer = dyn Fn(&mut Command) -> io::Result<Output> + Send + Sync;

/// What a start of a program is told, and what each start was
#[derive(Clone)]
struct Recorder {
    started: Arc<Mutex<Vec<Started>>>,
    answer: Arc<Answer>,
}

impl Recorder {
    fn answering(
        answer: impl Fn(&mut Command) -> io::Result<Output> + Send + Sync + 'static,
    ) -> Self {
        Self {
            started: Arc::default(),
            answer: Arc::new(answer),
        }
    }

    fn started(&self) -> Vec<Started> {
        self.started.lock().unwrap().clone()
    }
}

impl Run for Recorder {
    fn run(&self, command: &mut Command) -> io::Result<Output> {
        let program = PathBuf::from(command.get_program());
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        self.started.lock().unwrap().push((program, args));
        (self.answer)(command)
    }
}

fn exits(code: i32, stderr: &str) -> io::Result<Output> {
    Ok(Output {
        status: status(code),
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
    })
}

#[test]
fn the_projects_issue_and_pull_urls_open() {
    for url in [
        "https://github.com/shep-pm/kelpie/issues/146",
        "https://github.com/shep-pm/kelpie/pull/9",
        // GitHub reads a repo's name without regard to case
        "https://github.com/Shep-PM/Kelpie/issues/1",
    ] {
        assert_eq!(check(url, &repo()), Ok(()), "{url}");
    }
}

#[test]
fn another_repo_another_host_and_another_path_are_refused() {
    for url in [
        "https://github.com/shep-pm/other/issues/146",
        "https://github.com/shep-pm/kelpie-extra/issues/146",
        "http://github.com/shep-pm/kelpie/issues/146",
        "https://gist.github.com/shep-pm/kelpie/issues/146",
        "https://github.com.example.org/shep-pm/kelpie/issues/146",
        "https://github.com@example.org/shep-pm/kelpie/issues/146",
        "https://github.com/shep-pm/kelpie/issues",
        "https://github.com/shep-pm/kelpie/issues/",
        "https://github.com/shep-pm/kelpie/issues/0",
        "https://github.com/shep-pm/kelpie/issues/abc",
        "https://github.com/shep-pm/kelpie/issues/12/files",
        "https://github.com/shep-pm/kelpie/issues/12?x=1",
        "https://github.com/shep-pm/kelpie/issues/12#top",
        "https://github.com/shep-pm/kelpie/tree/main",
        "https://github.com/shep-pm/kelpie/issues/12'",
        "file:///etc/passwd",
        "--help",
        "",
    ] {
        let said = check(url, &repo()).expect_err(url);
        assert!(said.contains("shep-pm/kelpie"), "{url}: {said}");
    }
}

#[cfg(unix)]
#[test]
fn macos_hands_the_url_to_open_through_the_process_tree() {
    // `true` stands in for `open`: it exits at once, as `open` does once
    // LaunchServices has the URL
    let dir = std::env::temp_dir().join(format!("kelpie-link-{}", RunId::new().unwrap()));
    std::fs::create_dir_all(&dir).unwrap();
    let tree = Arc::new(ProcessTree::new(RunId::new().unwrap(), dir.join("run.json")).unwrap());
    let standin = ["/usr/bin/true", "/bin/true"]
        .into_iter()
        .find(|path| Path::new(path).exists())
        .expect("a `true` command");
    // Every command the recorder is given is started in the tree
    let recorder = Recorder::answering({
        let tree = Arc::clone(&tree);
        move |command| tree.output(command)
    });
    let programs = Programs {
        open: standin.into(),
        gdbus: "gdbus".into(),
    };

    let url = "https://github.com/shep-pm/kelpie/issues/146";
    let shown = open_with(&recorder, Os::MacOs, &programs, &repo(), url);

    assert_eq!(shown, Ok(LinkView::Opened));
    assert_eq!(
        recorder.started(),
        [(PathBuf::from(standin), vec![url.to_owned()])],
        "`open` got the URL and nothing else"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn linux_with_no_portal_copies_the_link_and_says_so() {
    let url = "https://github.com/shep-pm/kelpie/pull/9";
    let programs = Programs::default();

    // gdbus is there, and no portal answers
    let silent = Recorder::answering(|_| {
        exits(
            1,
            "Error: GDBus.Error:org.freedesktop.DBus.Error.ServiceUnknown: The name org.freedesktop.portal.Desktop was not provided by any .service files",
        )
    });
    let shown = open_with(&silent, Os::Linux, &programs, &repo(), url).unwrap();
    let LinkView::Copy {
        url: copied,
        message,
    } = shown
    else {
        panic!("expected the link to be copied, got {shown:?}");
    };
    assert_eq!(copied, url);
    assert!(
        message.contains("portal") && message.contains("copied"),
        "{message}"
    );

    // there is no gdbus at all
    let missing = Recorder::answering(|_| Err(io::Error::from(io::ErrorKind::NotFound)));
    assert!(matches!(
        open_with(&missing, Os::Linux, &programs, &repo(), url),
        Ok(LinkView::Copy { .. })
    ));

    // and where the portal answers, the link opens, over the session bus
    let portal = Recorder::answering(|_| exits(0, ""));
    assert_eq!(
        open_with(&portal, Os::Linux, &programs, &repo(), url),
        Ok(LinkView::Opened)
    );
    let started = portal.started();
    let (program, args) = &started[0];
    assert_eq!(program, Path::new("gdbus"));
    for part in [
        "call",
        "--session",
        "org.freedesktop.portal.Desktop",
        "org.freedesktop.portal.OpenURI.OpenURI",
        "'https://github.com/shep-pm/kelpie/pull/9'",
    ] {
        assert!(args.iter().any(|arg| arg == part), "{part} in {args:?}");
    }
}

#[test]
fn xdg_open_is_never_run() {
    let url = "https://github.com/shep-pm/kelpie/issues/146";
    let programs = Programs::default();
    for os in [Os::MacOs, Os::Linux, Os::Windows] {
        for answer in [exits(0, ""), exits(1, "no"), Err(io::Error::other("no"))] {
            let text = format!("{answer:?}");
            let recorder = Recorder::answering(move |_| match &answer {
                Ok(output) => Ok(output.clone()),
                Err(why) => Err(io::Error::new(why.kind(), why.to_string())),
            });

            let _ = open_with(&recorder, os, &programs, &repo(), url);

            for (program, args) in recorder.started() {
                let name = program.file_name().unwrap().to_string_lossy().into_owned();
                assert!(
                    matches!(name.as_str(), "open" | "gdbus"),
                    "{os:?} with {text} ran {name}"
                );
                assert!(
                    args.iter().all(|arg| !arg.contains("xdg-open")),
                    "{os:?} with {text} passed xdg-open in {args:?}"
                );
            }
        }
    }
}
