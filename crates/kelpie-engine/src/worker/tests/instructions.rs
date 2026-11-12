//! What a worker's first prompt says: the instructions with their placeholders filled in

use super::*;

#[test]
fn the_instructions_carry_the_base_branch_and_the_hand_over_file() {
    let rig = Rig::new("instructions");
    let binding = binding_on("Run `{kelpie}` on `origin/{base}` and write {pull_request}.");
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

    let prompt = agent.params("session/prompt")[0]["prompt"][0]["text"]
        .as_str()
        .expect("the prompt holds no text")
        .to_owned();
    assert!(prompt.contains("origin/main"), "{prompt:?}");
    assert!(
        prompt.contains(&rig.paths.pull_request_file("74").display().to_string()),
        "{prompt:?}"
    );
    assert!(
        !prompt.contains("{base}")
            && !prompt.contains("{kelpie}")
            && !prompt.contains("{pull_request}"),
        "no placeholder reaches the worker: {prompt:?}"
    );
}
