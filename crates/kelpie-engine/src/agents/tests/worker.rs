//! The built-in worker and the worker files that extend it

use super::*;

#[test]
fn the_built_in_worker_resolves_with_no_file_of_the_maintainers() {
    let dir = Dir::new();

    let worker = dir.load().worker().unwrap();

    assert_eq!(worker.role.value, Role::Worker);
    assert_eq!(worker.role.from, Origin::Shipped("worker".to_owned()));
    // Kelpie names no harness and no model (ADR 0006)
    assert_eq!(worker.harness.value, None);
    assert_eq!(worker.model.value, None);
    assert_eq!(worker.effort.value, None);
    let text = worker.body.value.unwrap();
    assert!(text.contains("Working as a kelpie worker"), "{text}");
    assert!(text.contains("{kelpie} lease run cargo-test"), "{text}");
    assert_eq!(worker.body.from, Origin::Shipped("worker".to_owned()));
    assert_eq!(dir.load().problems().count(), 0);
}

#[test]
fn a_project_worker_file_extends_kelpie_worker() {
    let dir = Dir::new();
    dir.put(
        "strict-worker.md",
        "---\nextends: kelpie:worker\nmodel: small\neffort: low\n---\nAlways run the linter.\n",
    );
    dir.put(
        "quiet-worker.md",
        "---\nextends: kelpie:worker\nbody: append\n---\nAsk before any push.\n",
    );
    let agents = dir.load();
    assert_eq!(
        agents.problems().count(),
        0,
        "{:?}",
        agents.problems().next()
    );

    let strict = agents.get("strict-worker").unwrap();
    assert_eq!(strict.role.value, Role::Worker);
    assert_eq!(strict.role.from, Origin::Shipped("worker".to_owned()));
    assert_eq!(model(strict), Some("small"));
    assert_eq!(strict.model.from, Origin::File("strict-worker".to_owned()));
    assert_eq!(effort(strict), Some("low"));
    // Its own body replaces kelpie's
    assert_eq!(body(strict), Some("Always run the linter."));

    // `body: append` keeps kelpie's and adds the file's
    let quiet = agents.get("quiet-worker").unwrap();
    let text = body(quiet).unwrap();
    assert!(text.starts_with("# Working as a kelpie worker"), "{text}");
    assert!(text.ends_with("Ask before any push."), "{text}");
    assert_eq!(model(quiet), None);

    // A global `worker.md` that extends nothing builds on kelpie's too
    dir.put("worker.md", "---\nmodel: global-model\n---\n");
    let global = dir.load().worker().unwrap();
    assert_eq!(model(&global), Some("global-model"));
    assert_eq!(
        body(&global),
        Agents::built_in("worker").unwrap().body.value.as_deref()
    );
}
