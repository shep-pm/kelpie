use super::{Backend, LoginError, Store};

#[test]
fn a_login_is_set_read_and_cleared() {
    let store = Store::memory();
    assert!(!store.status("claude").unwrap());
    assert_eq!(store.get("claude").unwrap(), None);

    store.set("claude", "sk-ant-oat01-secret").unwrap();

    assert!(store.status("claude").unwrap());
    assert_eq!(
        store.get("claude").unwrap().as_deref(),
        Some("sk-ant-oat01-secret")
    );

    store.clear("claude").unwrap();

    assert!(!store.status("claude").unwrap());
    assert_eq!(store.get("claude").unwrap(), None);
}

#[test]
fn each_harness_has_a_login_of_its_own() {
    let store = Store::memory();

    store.set("claude", "one").unwrap();
    store.set("goose", "two").unwrap();
    store.clear("claude").unwrap();

    assert!(!store.status("claude").unwrap());
    assert_eq!(store.get("goose").unwrap().as_deref(), Some("two"));
}

#[test]
fn clearing_a_login_that_is_not_set_is_fine() {
    Store::memory().clear("claude").unwrap();
}

#[test]
fn a_pasted_login_loses_its_surrounding_whitespace() {
    let store = Store::memory();

    store.set("claude", "  token\n").unwrap();

    assert_eq!(store.get("claude").unwrap().as_deref(), Some("token"));
}

#[test]
fn a_blank_login_is_refused_and_leaves_the_old_one() {
    let store = Store::memory();
    store.set("claude", "token").unwrap();

    let error = store.set("claude", " \n").unwrap_err();

    assert_eq!(error, LoginError::Blank);
    assert_eq!(store.get("claude").unwrap().as_deref(), Some("token"));
}

/// Writes `value` under `harness` the way another program could: past `Store::set`
fn written_outside(store: &Store, harness: &str, value: &str) {
    let Backend::Memory(logins) = &store.backend else {
        unreachable!("a memory store");
    };
    logins
        .lock()
        .unwrap()
        .insert(harness.to_owned(), value.to_owned());
}

#[test]
fn a_login_written_outside_kelpie_is_read_as_set_would_keep_it() {
    let store = Store::memory();

    written_outside(&store, "claude", "token\n");
    assert_eq!(store.get("claude").unwrap().as_deref(), Some("token"));

    for blank in ["", " \n"] {
        written_outside(&store, "claude", blank);
        assert_eq!(store.get("claude").unwrap(), None, "{blank:?}");
        assert!(!store.status("claude").unwrap(), "{blank:?}");
    }
}

#[test]
fn a_clone_shares_the_logins() {
    let store = Store::memory();

    store.clone().set("claude", "token").unwrap();

    assert!(store.status("claude").unwrap());
}

#[test]
fn a_store_keeps_its_logins_out_of_debug() {
    // A lazy `derive(Debug)` would print the map, and with it every login
    let store = Store::memory();
    store.set("claude", "sk-ant-oat01-secret").unwrap();

    assert_eq!(format!("{store:?}"), "Store { memory }");
    assert_eq!(format!("{:?}", Store::keychain()), "Store { keychain }");
}

#[test]
fn a_keychain_error_never_carries_a_stored_value() {
    let bytes = b"sk-ant-oat01-secret".to_vec();
    for error in [
        keyring::Error::BadEncoding(bytes.clone()),
        keyring::Error::BadDataFormat(bytes, "sk-ant-oat01-secret".into()),
    ] {
        let error = LoginError::from(error);

        assert!(!format!("{error} {error:?}").contains("secret"), "{error}");
    }
}

/// Round-trips one login through the real OS keychain.
///
/// CI has no keychain, so this runs by hand on macOS and Linux. It uses a
/// harness id of its own, so it leaves a person's real logins alone.
#[ignore = "needs a real OS keychain, which CI does not have"]
#[test]
fn a_real_keychain_round_trips() {
    let store = Store::keychain();
    let harness = "kelpie-test-login";
    store.clear(harness).unwrap();
    assert!(!store.status(harness).unwrap());

    store.set(harness, "a-test-token").unwrap();

    assert!(store.status(harness).unwrap());
    assert_eq!(store.get(harness).unwrap().as_deref(), Some("a-test-token"));
    store.clear(harness).unwrap();
    assert!(!store.status(harness).unwrap());
}
