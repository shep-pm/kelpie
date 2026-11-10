//! Each worker's lease socket, from the start that gives it to the opener
//!
//! One socket per session, in its state folder, named in its environment so a
//! `kelpie lease run` it starts can reach kelpie ([`crate::lease`]).

use std::ffi::OsString;

use super::*;
use crate::lease::SOCKET_VAR;
use crate::settings::{ConfigDir, Settings};
use crate::tools::Door;

/// kelpie's leases, with one place for `cargo-test`, in the rig's data folder
fn leases(rig: &Rig) -> Leases {
    let config = rig.paths.data_dir.join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("settings.json"),
        r#"{"lease-capacity": {"cargo-test": 1}}"#,
    )
    .unwrap();
    Leases::new(&Settings::load(&ConfigDir::at(&config), "web").unwrap())
}

/// The socket a started worker was given, as its environment names it
fn socket_of(named: &Mutex<Vec<(OsString, OsString)>>) -> Door {
    let (name, value) = named.lock().unwrap().remove(0);
    assert_eq!(name, SOCKET_VAR, "the lease socket is named by");
    Door::parse(&value.to_string_lossy()).expect("the lease socket is a socket")
}

#[test]
fn each_worker_gets_a_lease_socket_of_its_own_named_in_its_environment() {
    let rig = Rig::new("lease");
    let leases = leases(&rig);
    let agent = FakeAgent::new().turn(Turn::new().text("done"));
    let binding = binding();
    let named = Arc::new(Mutex::new(Vec::<(OsString, OsString)>::new()));
    let seen = Arc::clone(&named);
    let open = |opening: &Opening<'_>| -> Result<Session, String> {
        let lease = opening
            .lease
            .as_ref()
            .expect("a worker started with leases has a lease socket");
        seen.lock().unwrap().push(lease.env());
        Ok(on(&agent, opening))
    };

    // Each worker is kept running while its socket is looked at: the socket ends
    // with the session
    let mut running = Vec::new();
    let mut doors = Vec::new();
    for id in ["74", "75"] {
        let (sink, heard) = events();
        let started = start(
            &rig.project_with(&binding, &open, 2, Some(&leases)),
            &ticket(id, "Do the thing"),
            sink,
        )
        .unwrap();
        assert!(matches!(started, Started::Running { .. }), "{started:?}");
        until_turn_ends(&heard);
        running.push(started);
        doors.push(socket_of(&named));
    }

    assert_ne!(doors[0], doors[1], "one lease socket per session");
    for door in &doors {
        #[cfg(target_os = "macos")]
        {
            let socket = door.socket().expect("macOS binds a Unix socket");
            assert!(
                socket.exists(),
                "{socket:?} is the socket kelpie bound for this worker"
            );
        }
        #[cfg(target_os = "linux")]
        {
            let address = door.tcp().expect("Linux binds a loopback address");
            assert!(address.ip().is_loopback(), "{address}");
        }
    }
}
