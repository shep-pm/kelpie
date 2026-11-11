//! Tests for what the PM's last start found about its sandbox

use kelpie_engine::sandbox::Refusal;

use super::{RefusalView, SandboxSetup, SandboxView};

#[test]
fn a_start_that_found_no_refusal_says_none() {
    let setup = SandboxSetup::default();
    assert_eq!(setup.view(), SandboxView { refusal: None });

    setup.record(None);
    assert_eq!(setup.view(), SandboxView { refusal: None });
}

#[test]
fn a_refusal_is_kept_until_a_later_start_finds_none() {
    let setup = SandboxSetup::default();
    setup.record(Some(Refusal::NoCapabilities));
    assert!(setup.view().refusal.is_some());
    assert!(
        setup.view().refusal.is_some(),
        "a look at the view forgets nothing"
    );

    setup.record(None);
    assert_eq!(setup.view(), SandboxView { refusal: None });
}

#[test]
fn the_two_refusals_are_told_apart_in_the_view() {
    let setup = SandboxSetup::default();

    setup.record(Some(Refusal::NoCapabilities));
    assert_eq!(setup.view().refusal, Some(RefusalView::NoCapabilities));

    setup.record(Some(Refusal::NoNamespaces));
    assert_eq!(setup.view().refusal, Some(RefusalView::NoNamespaces));

    // The webview reads these two strings
    let wire = serde_json::to_string(&setup.view()).unwrap();
    assert_eq!(wire, r#"{"refusal":"no_namespaces"}"#);
}
