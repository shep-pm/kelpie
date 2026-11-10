//! The check pass: find a newer version in the index, install it, probe it, and
//! record what was found
//!
//! `bridge/check.rs`

use std::ffi::OsString;
use std::io;

use agent_client_protocol::{Channel, DynConnectTo};

use super::*;
use crate::session::fake::FakeAgent;
use crate::session::{Harness, Launch, Link};
use crate::settings::BridgeUpdates;
/// Check the bridge against the registry `server` serves, probing with `probe`
fn checked_with(
    dir: &Dir,
    server: &Server,
    url: &str,
    probe: &dyn Fn(&Harness) -> Result<(), ProbeError>,
) -> CheckOutcome {
    let platforms = platforms_for(server, TAR);
    check::checked(
        url,
        dir.data(),
        &pin(&platforms),
        BridgeUpdates::RegistryAutomatic,
        probe,
    )
}
#[test]

fn a_bridge_missing_from_the_index_keeps_what_it_has() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);
    let platforms = platforms_for(&server, TAR);
    let mut other = pin(&platforms);
    other.registry_id = "not-in-the-index";

    let outcome = check::checked(
        &server.url(REGISTRY),
        dir.data(),
        &other,
        BridgeUpdates::RegistryAutomatic,
        &|_| panic!("nothing was installed to probe"),
    );

    assert_eq!(
        outcome,
        CheckOutcome::Unlisted {
            in_use: version(VERSION)
        }
    );
    assert!(!dir.data().join("bridges").exists(), "nothing was written");
}

#[test]

fn a_newer_version_is_installed_probed_and_ready() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);
    let probed = std::cell::RefCell::new(Vec::new());

    let outcome = checked_with(&dir, &server, &server.url(REGISTRY), &|harness| {
        probed.borrow_mut().push(harness.program.clone());
        Ok(())
    });

    assert_eq!(
        outcome,
        CheckOutcome::Ready {
            version: version("0.102.0")
        }
    );
    let installed = dir
        .data()
        .join("bridges")
        .join(ID)
        .join("0.102.0")
        .join(CMD);
    assert_eq!(
        *probed.borrow(),
        std::slice::from_ref(&installed),
        "the new version was probed once"
    );
    assert_eq!(fs::read_to_string(&installed).unwrap(), COMMAND_TEXT);
    let platforms = [(this_platform(), unfetched(TAR))];
    let versions = Versions::load(dir.data(), &pin(&platforms)).unwrap();
    assert_eq!(versions.in_use, version(VERSION), "nothing swaps here");
    assert_eq!(versions.ready, Some(version("0.102.0")));
    assert!(versions.held_back.is_empty());
}

#[test]

fn the_probe_starts_the_bridge_with_its_args() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);
    let probed = std::cell::RefCell::new(Vec::<Harness>::new());
    // `check` installs what the index names, so this pin's own archive is never fetched
    let platforms = [(this_platform(), unfetched(GOOSE_TAR))];

    let outcome = check::checked(
        &server.url(REGISTRY),
        dir.data(),
        &goose_pin(&platforms),
        BridgeUpdates::RegistryAutomatic,
        &|harness| {
            probed.borrow_mut().push(harness.clone());
            let mut bridge = AnswersWithArg {
                harness: harness.clone(),
                agent: FakeAgent::new().can_resume(),
            };
            probe::probed_within(&mut bridge, &[Capability::ResumeSession], ANSWER)
        },
    );

    assert_eq!(
        outcome,
        CheckOutcome::Ready {
            version: version(GOOSE_VERSION)
        },
        "the bridge was held back"
    );
    let probed = probed.borrow();
    let harness = probed.first().expect("the new version was probed");
    assert_eq!(
        harness.args,
        [OsString::from(ACP)],
        "the bridge was started bare"
    );
    assert_eq!(
        harness.env,
        [(OsString::from(GOOSE_PROVIDER), OsString::from(OLLAMA))],
        "the bridge was started with no env of its own"
    );
    assert_eq!(
        harness.program,
        dir.data()
            .join("bridges")
            .join(GOOSE)
            .join(GOOSE_VERSION)
            .join(GOOSE_CMD),
        "the bridge that was probed is not the one that was installed"
    );
}

/// A pin for the fixture's bridge that carries its args, at a version older than the index's
fn goose_pin(platforms: &[(Platform, Download)]) -> Pin<'_> {
    Pin {
        id: GOOSE,
        registry_id: GOOSE,
        version: "1.1.0",
        platforms,
    }
}

/// A bridge that answers `initialize` only when it was started with `ACP`
///
/// Started without it, it closes its end of the connection at once and says
/// nothing, which the probe reads as the bridge ending before it answered.
struct AnswersWithArg {
    harness: Harness,
    agent: FakeAgent,
}

impl Launch for AnswersWithArg {
    fn launch(&mut self) -> io::Result<Link> {
        if self.harness.args != [OsString::from(ACP)] {
            let (client_side, agent_side) = Channel::duplex();
            drop(agent_side);
            return Ok(Link {
                transport: DynConnectTo::new(client_side),
                explain: Box::new(|| Some(NO_ARG.to_owned())),
                stop: Box::new(|_| Some(NO_ARG.to_owned())),
            });
        }
        self.agent.launch()
    }
}

#[test]

fn a_version_failing_the_probe_is_held_back_with_its_reason() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);

    let outcome = checked_with(&dir, &server, &server.url(REGISTRY), &|_| {
        Err(ProbeError::Missing {
            capability: Capability::ResumeSession,
        })
    });

    let CheckOutcome::HeldBack {
        version: held,
        reason,
    } = outcome
    else {
        panic!("{outcome:?}")
    };
    assert_eq!(held, version("0.102.0"));
    assert!(reason.contains("session/resume"), "{reason}");
    let platforms = [(this_platform(), unfetched(TAR))];
    let versions = Versions::load(dir.data(), &pin(&platforms)).unwrap();
    assert_eq!(versions.ready, None);
    assert_eq!(
        versions.held_back,
        [Rejected {
            version: version("0.102.0"),
            reason
        }]
    );
}

#[test]

fn a_bad_version_is_not_tried_again() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);
    let platforms = [(this_platform(), unfetched(TAR))];
    let mut versions = Versions::load(dir.data(), &pin(&platforms)).unwrap();
    versions.bad.push(Rejected {
        version: version("0.102.0"),
        reason: "failed to resume".to_string(),
    });
    versions.save(dir.data(), ID).unwrap();

    let outcome = checked_with(&dir, &server, &server.url(REGISTRY), &|_| {
        panic!("a bad version was probed again")
    });

    assert_eq!(outcome, CheckOutcome::Current);
    assert_eq!(server.served(), 0, "a bad version was downloaded again");
    assert!(!dir.data().join("bridges").join(ID).join("0.102.0").exists());
}

#[test]

fn a_version_older_than_kelpies_own_pin_is_no_update() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);
    let platforms = platforms_for(&server, TAR);
    let mut versions = Versions::load(dir.data(), &pin(&platforms)).unwrap();
    versions.in_use = version("0.101.1");
    versions.save(dir.data(), ID).unwrap();
    let mut newer_pin = pin(&platforms);
    newer_pin.version = "0.103.0";

    let outcome = check::checked(
        &server.url(REGISTRY),
        dir.data(),
        &newer_pin,
        BridgeUpdates::RegistryAutomatic,
        &|_| panic!("a version older than kelpie's pin was probed"),
    );

    assert_eq!(outcome, CheckOutcome::Current);
    assert_eq!(server.served(), 0, "an older version was downloaded");
    assert_eq!(
        Versions::load(dir.data(), &newer_pin).unwrap().ready,
        None,
        "an older version was recorded as ready"
    );
}

#[test]

fn kelpies_pin_fetches_nothing() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);
    let platforms = platforms_for(&server, TAR);

    let outcome = check::checked(
        &server.url(REGISTRY),
        dir.data(),
        &pin(&platforms),
        BridgeUpdates::KelpiesPin,
        &|_| panic!("kelpie's pin probed a version"),
    );

    assert_eq!(outcome, CheckOutcome::Pinned);
    assert_eq!(server.served(), 0);
    assert!(!dir.data().join("bridges").exists());
}
#[test]

fn a_failed_fetch_changes_nothing() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);

    let outcome = checked_with(&dir, &server, &server.url("missing.json"), &|_| {
        panic!("nothing was installed to probe")
    });

    let CheckOutcome::Failed { reason } = outcome else {
        panic!("{outcome:?}")
    };
    assert!(reason.contains("missing.json"), "{reason}");
    assert!(
        !dir.data().join("bridges").exists(),
        "a failed check wrote something"
    );
}
