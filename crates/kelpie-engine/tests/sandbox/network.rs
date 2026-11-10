//! The sandbox's network: what a running sandbox dials, and what a host the proxy
//! refused reaches once `request_network` answers
//!
//! A child of `tests/sandbox.rs`, which is its own `main`: the dial is this binary
//! run again with [`DIAL`] (crabpot's Linux helper is this binary run again too).

use std::net::{SocketAddr, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use kelpie_engine::sandbox::{Reads, Sandbox};
use kelpie_engine::tools::Reach;

use super::host::working_sandbox;
use super::support::{Scratch, sh};

/// The argument that makes this binary dial an address and say whether it got through
pub(super) const DIAL: &str = "--dial";

/// Exit 0 when `address`, a socket's path or `host:port`, answers, and 1 when not
pub(super) fn dial(address: &str) -> i32 {
    let reached = if Path::new(address).is_absolute() {
        UnixStream::connect(address).is_ok()
    } else {
        address.parse::<SocketAddr>().is_ok_and(|address| {
            TcpStream::connect_timeout(&address, Duration::from_secs(3)).is_ok()
        })
    };
    i32::from(!reached)
}

/// `curl`'s answer to a CONNECT through the sandbox's proxy to `https://{host}/`
fn connect_code(sandbox: &Sandbox, host: &str) -> String {
    let out = sh(
        sandbox,
        &format!("curl -sS -m 20 -o /dev/null -w '%{{http_connect}}' https://{host}/"),
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A host the proxy refused is, once `request_network` is allowed, no longer
/// refused by the same sandbox, still running
pub(super) fn an_allowed_host_is_reached_by_a_running_sandbox() {
    let scratch = Scratch::new("widen");
    let exe = std::env::current_exe().unwrap();
    let Some(sandbox) = working_sandbox(
        &scratch,
        "an_allowed_host_is_reached_by_a_running_sandbox",
        &exe,
        Reads::Project,
    ) else {
        return;
    };
    assert_eq!(
        connect_code(&sandbox, "example.com"),
        "403",
        "the proxy did not refuse the host before the answer"
    );
    let network = sandbox.network();

    assert!(network.allow("example.com"), "the sandbox took the host");

    // Let through: a 200 with a network, and no 403 without one
    let code = connect_code(&sandbox, "example.com");
    assert_ne!(code, "403", "the host was still refused after the answer");
    assert_eq!(
        connect_code(&sandbox, "example.org"),
        "403",
        "only the host asked for was let through"
    );
}
