//! The worker's credential on GitHub: minted for one turn, in a folder of its own
//!
//! `worker::login`, and the environment `worker::start` builds from it, as each
//! start hands them to the opener

use super::*;

/// The variables each start gave its opener, in order
fn envs(held: &Mutex<Vec<Vec<(String, String)>>>) -> Vec<Vec<(String, String)>> {
    held.lock().unwrap().clone()
}

/// `name` in `env`, `None` where kelpie sets no such variable
fn in_env(env: &[(String, String)], name: &str) -> Option<String> {
    env.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
}

/// The token file the worker's `gh` reads, as kelpie wrote it
fn hosts(state_dir: &Path) -> String {
    std::fs::read_to_string(login::config_dir(state_dir).join("hosts.yml")).unwrap()
}

/// The first turn's prompt, which carries the instructions and the ticket's text
fn prompt(agent: &FakeAgent) -> String {
    agent.params("session/prompt")[0]["prompt"][0]["text"]
        .as_str()
        .expect("the prompt holds no text")
        .to_owned()
}

/// The prompt of the turn that ran last, for a test that starts a ticket twice
fn last_prompt(agent: &FakeAgent) -> String {
    agent
        .params("session/prompt")
        .last()
        .and_then(|turn| turn["prompt"][0]["text"].as_str())
        .expect("no turn prompted the agent")
        .to_owned()
}

// The worker's credential on GitHub: one repo, minted for this turn alone, in a
// folder of its own that git's credential helper answers from. No variable
// carries it, so a harness that strips token-shaped names, and every process
// that reads another's environment, still sees nothing.
#[test]
fn a_worker_pushes_with_a_token_for_its_one_repo_minted_fresh_for_each_turn() {
    let mut rig = Rig::new("worker token");
    rig.on_github("shep-pm/koji");
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    rig.tokens = Some(github.tokens(home.path()));

    let binding = binding();
    let agent = FakeAgent::new()
        .turn(Turn::new().text("first turn"))
        .turn(Turn::new().text("second turn"));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let open = |opening: &Opening<'_>| -> Result<Session, String> {
        seen.lock().unwrap().push(opening.env.clone());
        Ok(on(&agent, opening))
    };

    let (sink, heard) = events();
    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    let Started::Running { state_dir, .. } = started else {
        panic!("{started:?}");
    };
    until_turn_ends(&heard);
    let first = hosts(&state_dir);
    assert!(first.contains("oauth_token: ghs_test1"), "{first}");
    assert!(first.contains("user: x-access-token"), "{first}");

    let (sink, heard) = events();
    let again = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    assert!(matches!(again, Started::Running { .. }), "{again:?}");
    until_turn_ends(&heard);

    let second = hosts(&state_dir);
    assert!(second.contains("oauth_token: ghs_test2"), "{second}");
    assert!(!second.contains("ghs_test1"), "{second}");
    assert_eq!(
        github.asked().len(),
        4,
        "each turn mints its own, and kelpie mints no token of its own here"
    );

    let envs = envs(&seen);
    assert_eq!(envs.len(), 2, "each turn gets its own environment");
    for env in &envs {
        let build = rig.paths.build_dir("74");
        let dir = login::config_dir(&state_dir);
        assert_eq!(
            in_env(env, "CARGO_TARGET_DIR").as_deref(),
            Some(build.to_string_lossy().as_ref()),
            "{env:?}"
        );
        assert_eq!(
            in_env(env, "GH_CONFIG_DIR").as_deref(),
            Some(dir.to_string_lossy().as_ref()),
            "the worker's `gh` reads its own folder, not the maintainer's: {env:?}"
        );
        assert_eq!(
            in_env(env, "GIT_TERMINAL_PROMPT").as_deref(),
            Some("0"),
            "{env:?}"
        );
        assert!(
            in_env(env, "GIT_CONFIG_PARAMETERS")
                .unwrap()
                .contains("!gh auth git-credential"),
            "git answers github.com from the worker's own login: {env:?}"
        );
        assert!(
            env.iter().all(|(_, value)| !value.contains("ghs_test")),
            "no variable carries the token: {env:?}"
        );
    }
}

// An App kelpie registered that the repo does not cover leaves the worker with
// nothing to push with. It runs, says so, and git is pointed at no helper at
// all, so nothing the maintainer logged into is ever asked.
#[test]
fn a_worker_with_no_app_on_its_repo_is_told_it_cannot_push() {
    let mut rig = Rig::new("no app");
    rig.on_github("shep-pm/koji");
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    // Registered, and installed on another repo alone
    github.registered(home.path(), "other/koji");
    rig.tokens = Some(github.tokens(home.path()));

    let binding = binding();
    let agent = FakeAgent::new().turn(Turn::new().text("done"));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let open = |opening: &Opening<'_>| -> Result<Session, String> {
        seen.lock().unwrap().push(opening.env.clone());
        Ok(on(&agent, opening))
    };
    let (sink, heard) = events();

    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    let Started::Running { state_dir, .. } = started else {
        panic!("{started:?}");
    };
    until_turn_ends(&heard);

    let prompt = prompt(&agent);
    assert!(
        prompt.contains("no credential for this repo") && prompt.contains("git push"),
        "{prompt:?}"
    );
    assert!(
        prompt.contains("no GitHub App installed on it"),
        "the worker names the cause it can actually report: {prompt:?}"
    );
    assert!(
        !login::config_dir(&state_dir).join("hosts.yml").exists(),
        "no login is written where there is no token"
    );
    let env = &envs(&seen)[0];
    assert_eq!(
        in_env(env, "GIT_CONFIG_PARAMETERS").as_deref(),
        Some(login::NO_HELPER),
        "the maintainer's helpers are emptied, and github.com is answered by nothing: {env:?}"
    );
}

// A turn with no credential does not run on the token the turn before it left.
// The state folder outlives its turn, `GH_CONFIG_DIR` names it whatever the next
// turn can push with, and a minted token is live until its hour is up, so an App
// taken off the repo leaves a working one sitting where the worker can read it.
#[test]
fn a_turn_with_no_credential_loses_the_token_the_last_turn_left() {
    let mut rig = Rig::new("stale token");
    rig.on_github("shep-pm/koji");
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    rig.tokens = Some(github.tokens(home.path()));

    let binding = binding();
    let agent = FakeAgent::new()
        .turn(Turn::new().text("first turn"))
        .turn(Turn::new().text("second turn"));
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };

    let (sink, heard) = events();
    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    let Started::Running { state_dir, .. } = started else {
        panic!("{started:?}");
    };
    until_turn_ends(&heard);
    let first = hosts(&state_dir);
    assert!(first.contains("oauth_token: ghs_test1"), "{first}");

    // The App comes off the repo, and the same state folder comes back
    github.uninstall("shep-pm/koji");
    let (sink, heard) = events();
    let again = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    let Started::Running { .. } = again else {
        panic!("{again:?}");
    };
    until_turn_ends(&heard);

    let dir = login::config_dir(&state_dir);
    for name in ["hosts.yml", "config.yml"] {
        assert!(
            !dir.join(name).exists(),
            "{name} still holds the last turn's login"
        );
    }
    let prompt = last_prompt(&agent);
    assert!(
        prompt.contains("no credential for this repo"),
        "the second turn is told it cannot push: {prompt:?}"
    );
}

// A machine whose `PATH` holds no `gh` has no credential helper for a worker's
// git at all. kelpie says so before the turn rather than leaving the push to fail
// inside it: nothing is minted, no `gh` folder is written, and the worker runs on
// the same terms as one whose repo no App covers.
#[test]
fn a_worker_on_a_machine_with_no_gh_is_told_it_cannot_push() {
    let mut rig = Rig::new("no gh");
    rig.on_github("shep-pm/koji");
    rig.gh = None;
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    // Installed on this very repo, and still no way to push with the token
    github.registered(home.path(), "shep-pm/koji");
    rig.tokens = Some(github.tokens(home.path()));

    let binding = binding();
    let agent = FakeAgent::new().turn(Turn::new().text("done"));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let open = |opening: &Opening<'_>| -> Result<Session, String> {
        seen.lock().unwrap().push(opening.env.clone());
        Ok(on(&agent, opening))
    };
    let (sink, heard) = events();

    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    let Started::Running { state_dir, .. } = started else {
        panic!("{started:?}");
    };
    until_turn_ends(&heard);

    assert!(
        github.asked().is_empty(),
        "no token was minted for a worker that could not push with it"
    );
    assert!(
        !login::config_dir(&state_dir).exists(),
        "no `gh` folder is written for a helper that is not there"
    );
    let env = &envs(&seen)[0];
    assert_eq!(
        in_env(env, "GIT_CONFIG_PARAMETERS").as_deref(),
        Some(login::NO_HELPER),
        "git is pointed at no helper, so nothing the maintainer logged into is asked: {env:?}"
    );
    assert_eq!(
        in_env(env, "GH_CONFIG_DIR").as_deref(),
        Some(login::config_dir(&state_dir).to_string_lossy().as_ref()),
        "a `gh` the worker does run still reads nowhere but its own: {env:?}"
    );
    let prompt = prompt(&agent);
    assert!(prompt.contains("no credential for this repo"), "{prompt:?}");
    assert!(
        prompt.contains("no `gh` is installed where its git runs"),
        "the worker is told the helper is what is missing: {prompt:?}"
    );
    assert!(
        !prompt.contains("GitHub App"),
        "and not an App it does have: {prompt:?}"
    );
}

// A repo kelpie cannot name on GitHub gets no token, whatever Apps it has: the
// token is for one repo, and this is not it.
#[test]
fn a_repo_that_is_not_github_gets_the_worker_no_token() {
    let mut rig = Rig::new("not github");
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    rig.tokens = Some(github.tokens(home.path()));

    let binding = binding();
    let agent = FakeAgent::new().turn(Turn::new().text("done"));
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let (sink, heard) = events();

    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    assert!(matches!(started, Started::Running { .. }), "{started:?}");
    until_turn_ends(&heard);

    assert!(
        github.asked().is_empty(),
        "no token was minted for any repo"
    );
    assert!(
        !login::config_dir(&rig.paths.state_dir("74")).exists(),
        "no `gh` folder is written for a repo kelpie cannot name"
    );
    let prompt = prompt(&agent);
    assert!(prompt.contains("no credential for this repo"), "{prompt:?}");
    assert!(
        prompt.contains("its remote is not GitHub"),
        "the worker names the remote, which is what the maintainer can check: {prompt:?}"
    );
}

// A token GitHub made hold more than pushing is never handed to a worker, and
// the start says why instead of running the turn with it.
#[test]
fn a_token_holding_more_than_pushing_refuses_the_start() {
    let mut rig = Rig::new("too broad");
    rig.on_github("shep-pm/koji");
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    github.grants_too("pull_requests", "write");
    rig.tokens = Some(github.tokens(home.path()));

    let binding = binding();
    let agent = FakeAgent::new().turn(Turn::new().text("done"));
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let (sink, heard) = events();

    let error = start(&rig.project(&binding, &open, 2), &ticket("74", "x"), sink)
        .expect_err("GitHub made the token hold pull_requests");
    assert!(
        matches!(error, StartError::Token(TokenError::TooBroad(_))),
        "{error:?}"
    );
    assert!(heard.try_recv().is_err(), "no turn ran with it");
    assert!(kept(&rig).is_empty(), "the refused start holds no slot");
    assert!(
        !login::config_dir(&rig.paths.state_dir("74")).exists(),
        "no login was written"
    );
    assert_eq!(agent.launches(), 0);
}
