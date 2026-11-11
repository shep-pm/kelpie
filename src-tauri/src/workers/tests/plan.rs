//! What a worker runs on, resolved from the settings

use std::time::Duration;

use kelpie_engine::agents::Origin;
use kelpie_engine::sandbox::Reads;
use kelpie_engine::settings::Field;
use serde_json::json;

use super::Rig;
use crate::workers::plan;

#[test]
fn the_plan_takes_slots_timeout_allowlist_and_reads_from_the_projects_settings() {
    let rig = Rig::new();
    rig.set(Field::WorkerSlots, json!(3));
    rig.set(Field::WorkerTurnTimeout, json!(15));
    rig.set(Field::NetworkAllowlist, json!(["docs.rs", "crates.io"]));
    rig.set(Field::ReadAnywhere, json!(true));

    let plan = plan(rig.config(), &rig.project).unwrap();

    assert_eq!(plan.slots, 3);
    assert_eq!(plan.turn_timeout, Duration::from_secs(15 * 60));
    assert_eq!(plan.network_allowlist, ["docs.rs", "crates.io"]);
    assert_eq!(plan.reads, Reads::Anywhere);
}

#[test]
fn the_plan_binding_is_the_worker_agent_with_the_projects_layers() {
    let rig = Rig::new();
    rig.set(Field::WorkerModel, json!("small"));

    let plan = plan(rig.config(), &rig.project).unwrap();

    assert_eq!(plan.binding.model.value.as_deref(), Some("small"));
    assert_eq!(plan.binding.model.from, Origin::Project);
    assert!(
        plan.binding
            .body
            .value
            .as_deref()
            .is_some_and(|body| body.contains("Working as a kelpie worker")),
        "the instructions are kelpie's own worker agent"
    );
    assert_eq!(plan.binding.body.from, Origin::Shipped("worker".to_owned()));
}
