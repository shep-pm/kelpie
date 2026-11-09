use std::fs;
use std::path::PathBuf;

use super::*;
use crate::run::RunId;
use crate::settings::{LayerFile, Settings};

const PARENT: &str = "---\nrole: pm\nharness: claude-code\nmodel: claude-opus-5-5\n\
                      effort: high\n---\nParent body.\n";

/// A config folder of its own for one test, never written over by another
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("kelpie-agents-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn config(&self) -> ConfigDir {
        ConfigDir::at(&self.0)
    }

    /// Write an agent file by hand, the way a person or a sync would
    fn put(&self, file: &str, text: &str) {
        let folder = self.0.join(FOLDER);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(file), text).unwrap();
    }

    fn put_all(&self, files: &[(&str, &str)]) {
        for (file, text) in files {
            self.put(file, text);
        }
    }

    fn load(&self) -> Agents {
        Agents::load(&self.config()).unwrap()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn load(files: &[(&str, &str)]) -> Agents {
    let dir = Dir::new();
    dir.put_all(files);
    dir.load()
}

/// What is wrong with the first file of `files` that is left out
fn refused(files: &[(&str, &str)]) -> String {
    let agents = load(files);
    agents
        .problems()
        .next()
        .expect("a file was left out")
        .to_string()
}

fn model(agent: &Agent) -> Option<&str> {
    agent.model.value.as_deref()
}

fn effort(agent: &Agent) -> Option<&str> {
    agent.effort.value.as_deref()
}

fn body(agent: &Agent) -> Option<&str> {
    agent.body.value.as_deref()
}

fn shipped_body() -> String {
    Agents::built_in("pm").unwrap().body.value.unwrap()
}

#[test]
fn kelpies_own_pm_is_a_pm_with_a_body_and_no_binding() {
    let pm = Agents::built_in("pm").unwrap();

    assert_eq!(pm.role.value, Role::Pm);
    assert_eq!(pm.role.from, Origin::Shipped("pm".to_owned()));
    assert_eq!(pm.harness.value, None);
    assert_eq!(pm.model.value, None);
    assert_eq!(pm.effort.value, None);
    let text = pm.body.value.unwrap();
    assert!(text.contains("decisions.md"), "{text}");
}

#[test]
fn a_missing_folder_holds_no_files() {
    let dir = Dir::new();

    let agents = dir.load();

    assert_eq!(agents.get("pm"), None);
    assert_eq!(agents.problems().count(), 0);
    assert!(!dir.0.join(FOLDER).exists(), "a read creates nothing");
}

#[test]
fn a_file_with_extends_and_no_body_takes_its_parents_body() {
    let agents = load(&[
        ("base.md", PARENT),
        ("light.md", "---\nextends: base\neffort: medium\n---\n"),
    ]);

    let light = agents.get("light").unwrap();

    assert_eq!(body(light), Some("Parent body."));
    assert_eq!(model(light), Some("claude-opus-5-5"));
    assert_eq!(effort(light), Some("medium"));
}

#[test]
fn one_with_a_body_uses_its_own() {
    let agents = load(&[
        ("base.md", PARENT),
        ("light.md", "---\nextends: base\n---\nMy own.\n"),
    ]);

    let light = agents.get("light").unwrap();

    assert_eq!(body(light), Some("My own."));
    assert_eq!(effort(light), Some("high"));
}

#[test]
fn keys_the_file_sets_replace_the_parents_and_the_rest_come_from_it() {
    let agents = load(&[
        (
            "base.md",
            "---\n# A comment.\nrole: pm\nharness: goose\nmodel: m1\neffort: high\n---\n",
        ),
        (
            "light.md",
            "---\nextends: base # the parent\n# Lighter.\neffort: low\nmodel: \"m2\"\n---\n",
        ),
    ]);

    let base = agents.get("base").unwrap();
    let light = agents.get("light").unwrap();

    assert_eq!(model(light), Some("m2"));
    assert_eq!(effort(light), Some("low"));
    assert_eq!(light.harness.value, Some(Harness::Goose));
    assert_eq!(light.harness.value, base.harness.value);
}

#[test]
fn harness_can_be_overridden() {
    let agents = load(&[
        ("base.md", PARENT),
        ("other.md", "---\nextends: base\nharness: goose\n---\n"),
    ]);

    let other = agents.get("other").unwrap();

    assert_eq!(other.harness.value, Some(Harness::Goose));
    assert_eq!(body(other), Some("Parent body."));
}

#[test]
fn a_merged_file_that_is_invalid_is_refused_naming_the_child() {
    let err = refused(&[
        ("base.md", PARENT),
        ("look.md", "---\nextends: base\nsecond_look: true\n---\n"),
    ]);

    assert!(err.contains("look.md"), "{err}");
    assert!(err.contains("second_look"), "{err}");
}

#[test]
fn a_missing_parent_is_refused_naming_the_file() {
    let err = refused(&[("light.md", "---\nextends: nobody\neffort: low\n---\n")]);

    assert!(err.contains("light.md"), "{err}");
    assert!(err.contains("nobody"), "{err}");
}

#[test]
fn a_bare_name_with_no_file_is_refused_as_naming_no_file_though_kelpie_ships_it() {
    let err = refused(&[("light.md", "---\nextends: pm\n---\n")]);

    assert!(err.contains("light.md"), "{err}");
    assert!(err.contains("`extends: pm` names no file"), "{err}");
}

#[test]
fn an_unknown_shipped_name_is_refused_naming_the_shipped_agents() {
    let err = refused(&[("light.md", "---\nextends: kelpie:nope\n---\n")]);

    assert!(err.contains("light.md"), "{err}");
    assert!(
        err.contains("`extends: kelpie:nope` names no shipped agent"),
        "{err}"
    );
    assert!(err.ends_with("kelpie ships: pm"), "{err}");
}

#[test]
fn a_stub_that_sets_a_key_keeps_the_shipped_body_and_takes_its_own_value() {
    let agents = load(&[(
        "pm.md",
        "---\nextends: kelpie:pm\nharness: claude-code\nmodel: mine\neffort: low\n---\n",
    )]);

    let stub = agents.get("pm").unwrap();

    assert_eq!(model(stub), Some("mine"));
    assert_eq!(body(stub), Some(shipped_body().as_str()));
    assert_eq!(stub.role.value, Role::Pm);
}

#[test]
fn a_cycle_is_refused_naming_the_file() {
    let err = refused(&[
        ("a.md", "---\nextends: b\n---\n"),
        ("b.md", "---\nextends: a\n---\n"),
    ]);

    assert!(err.contains("a.md"), "{err}");
    assert!(err.contains("circle"), "{err}");

    let err = refused(&[("me.md", "---\nextends: me\n---\n")]);

    assert!(err.contains("me.md"), "{err}");
    assert!(err.contains("circle"), "{err}");
}

#[test]
fn a_parent_that_extends_another_resolves() {
    let agents = load(&[
        ("base.md", PARENT),
        ("mid.md", "---\nextends: base\neffort: medium\n---\n"),
        ("leaf.md", "---\nextends: mid\nmodel: m3\n---\n"),
    ]);

    let leaf = agents.get("leaf").unwrap();

    assert_eq!(model(leaf), Some("m3"));
    assert_eq!(effort(leaf), Some("medium"));
    assert_eq!(body(leaf), Some("Parent body."));
}

#[test]
fn a_child_that_sorts_before_its_parent_resolves() {
    let agents = load(&[
        ("a-child.md", "---\nextends: z-parent\neffort: low\n---\n"),
        ("z-parent.md", PARENT),
    ]);

    let child = agents.get("a-child").unwrap();

    assert_eq!(model(child), Some("claude-opus-5-5"));
    assert_eq!(effort(child), Some("low"));
    assert_eq!(body(child), Some("Parent body."));
}

#[test]
fn a_shipped_agent_can_be_extended_by_a_file_of_another_name() {
    let agents = load(&[(
        "light-pm.md",
        "---\nextends: kelpie:pm\nharness: claude-code\nmodel: claude-opus-5-5\neffort: medium\n---\n",
    )]);

    let light = agents.get("light-pm").unwrap();

    assert_eq!(effort(light), Some("medium"));
    assert_eq!(body(light), Some(shipped_body().as_str()));
    assert_eq!(light.role.value, Role::Pm);
}

#[test]
fn a_bare_name_extends_the_file_of_that_name_and_never_the_shipped_agent() {
    let agents = load(&[
        (
            "pm.md",
            "---\nrole: pm\nharness: claude-code\nmodel: mine\neffort: low\n---\nMine.\n",
        ),
        ("light.md", "---\nextends: pm\neffort: medium\n---\n"),
    ]);

    let light = agents.get("light").unwrap();

    assert_eq!(model(light), Some("mine"));
    assert_eq!(effort(light), Some("medium"));
    assert_eq!(body(light), Some("Mine."));
}

#[test]
fn an_edit_to_the_parent_reaches_the_child_on_reload() {
    let dir = Dir::new();
    dir.put_all(&[
        ("base.md", PARENT),
        ("light.md", "---\nextends: base\neffort: medium\n---\n"),
    ]);
    dir.load();

    dir.put(
        "base.md",
        &PARENT
            .replace("claude-opus-5-5", "m9")
            .replace("Parent body.", "New body."),
    );
    let agents = dir.load();

    let light = agents.get("light").unwrap();
    assert_eq!(model(light), Some("m9"));
    assert_eq!(effort(light), Some("medium"));
    assert_eq!(body(light), Some("New body."));
}

#[test]
fn a_refusal_of_a_key_the_file_sets_names_the_files_own_line() {
    let err = refused(&[
        ("base.md", PARENT),
        (
            "light.md",
            "---\nextends: base\nharness: bogus\n---\nA body.\n",
        ),
    ]);

    assert!(err.contains("light.md"), "{err}");
    assert!(err.contains("`harness`"), "{err}");
    assert!(err.ends_with("at line 3, column 10"), "{err}");
}

#[test]
fn a_refusal_of_a_key_it_inherits_names_the_parent_and_no_line() {
    let err = refused(&[
        ("a-light.md", "---\nextends: z-base\nmodel: m\n---\n"),
        (
            "z-base.md",
            "---\nrole: pm\nharness: bogus\nmodel: m\n---\n",
        ),
    ]);

    assert!(err.contains("a-light.md"), "{err}");
    assert!(err.contains("`harness`"), "{err}");
    assert!(err.ends_with("inherited through `z-base`"), "{err}");
    assert!(!err.contains("line"), "{err}");
}

#[test]
fn a_quoted_parent_name_with_a_comment_after_it_resolves() {
    let agents = load(&[
        ("base.md", PARENT),
        (
            "light.md",
            "---\nextends: \"base\" # the parent\neffort: low\n---\n",
        ),
    ]);

    assert_eq!(effort(agents.get("light").unwrap()), Some("low"));
}

#[test]
fn a_quoted_parent_name_keeps_a_hash_inside_it() {
    let err = refused(&[("light.md", "---\nextends: \"a #b\"\n---\n")]);

    assert!(err.contains("`extends: a #b`"), "{err}");
}

#[test]
fn a_shipped_agent_that_extends_is_refused() {
    let shipped = "---\nextends: other\n---\n";
    let parent = |_: &str| {
        Ok(extends::ParentText::Shipped {
            name: "odd",
            text: shipped,
        })
    };
    let context = extends::Context {
        parent: &parent,
        implicit: &|_| None,
    };

    let err = extends::merged(
        Origin::File("light".to_owned()),
        "---\nextends: odd\n---\n",
        &context,
    )
    .unwrap_err();

    assert!(err.contains("not supported"), "{err}");
}

#[test]
fn a_stub_extending_a_shipped_agent_loads_its_frontmatter_and_body() {
    let agents = load(&[("pm.md", "---\nextends: kelpie:pm\n---\n")]);

    let stub = agents.get("pm").unwrap();
    let shipped = Agents::built_in("pm").unwrap();

    assert_eq!(stub.role.value, shipped.role.value);
    assert_eq!(stub.harness.value, shipped.harness.value);
    assert_eq!(stub.body.value, shipped.body.value);
}

#[test]
fn body_append_puts_the_files_body_after_the_parents_with_a_blank_line() {
    let agents = load(&[
        ("base.md", PARENT),
        (
            "light.md",
            "---\nextends: base\nbody: append\n---\nMy addition.\n",
        ),
    ]);

    let light = agents.get("light").unwrap();

    assert_eq!(body(light), Some("Parent body.\n\nMy addition."));
}

#[test]
fn body_append_with_no_body_of_its_own_takes_the_parents() {
    let agents = load(&[
        ("base.md", PARENT),
        ("light.md", "---\nextends: base\nbody: append\n---\n"),
    ]);

    assert_eq!(body(agents.get("light").unwrap()), Some("Parent body."));
}

#[test]
fn body_append_applies_at_each_step_of_a_chain_down_to_a_shipped_agent() {
    let agents = load(&[
        (
            "mid.md",
            "---\nextends: kelpie:pm\nbody: append\nmodel: m2\n---\nMiddle.\n",
        ),
        (
            "leaf.md",
            "---\nextends: mid\nbody: append\nmodel: m3\n---\nLeaf.\n",
        ),
    ]);
    let shipped = shipped_body();

    let mid = agents.get("mid").unwrap();
    let leaf = agents.get("leaf").unwrap();

    assert_eq!(body(mid), Some(format!("{shipped}\n\nMiddle.").as_str()));
    assert_eq!(
        body(leaf),
        Some(format!("{shipped}\n\nMiddle.\n\nLeaf.").as_str())
    );
    assert_eq!(model(leaf), Some("m3"));
}

#[test]
fn a_body_key_with_any_value_but_append_is_refused_with_its_line() {
    for value in ["replace", "\"\"", "true"] {
        let text = format!("---\nextends: base\neffort: low\nbody: {value}\n---\nA body.\n");

        let err = refused(&[("base.md", PARENT), ("light.md", &text)]);

        assert!(err.contains("light.md"), "{err}");
        assert!(err.contains("`body`"), "{err}");
        assert!(err.contains("`append`"), "{err}");
        assert!(err.ends_with("at line 4"), "{err}");
    }
}

#[test]
fn a_body_key_on_a_file_that_extends_nothing_is_refused() {
    let err = refused(&[("light.md", &PARENT.replace("effort: high", "body: append"))]);

    assert!(err.contains("light.md"), "{err}");
    assert!(err.contains("body"), "{err}");
}

#[test]
fn the_line_of_a_later_refusal_holds_with_body_in_the_frontmatter() {
    let err = refused(&[
        ("base.md", PARENT),
        (
            "light.md",
            "---\nextends: base\nbody: append\nharness: bogus\n---\nA body.\n",
        ),
    ]);

    assert!(err.ends_with("at line 4, column 10"), "{err}");
}

#[test]
fn a_body_key_set_twice_is_refused_with_the_line_of_the_second() {
    for second in ["append", "replace"] {
        let text = format!("---\nextends: base\nbody: append\nbody: {second}\n---\nA body.\n");

        let err = refused(&[("base.md", PARENT), ("light.md", &text)]);

        assert!(err.contains("light.md"), "{err}");
        assert!(err.contains("`body`"), "{err}");
        assert!(err.ends_with("at line 4"), "{err}");
    }
}

#[test]
fn a_refusal_from_inside_a_parent_file_names_that_file_and_not_the_child() {
    let parent = "---\nextends: kelpie:pm\nbody: bogus\n---\n";

    let err = refused(&[("a.md", "---\nextends: z\n---\n"), ("z.md", parent)]);

    assert!(err.contains("a.md: in `z.md`: "), "{err}");
    assert!(err.contains("`body: bogus`"), "{err}");

    let twice = "---\nextends: kelpie:pm\nbody: append\nbody: append\n---\n";

    let err = refused(&[("a.md", "---\nextends: z\n---\n"), ("z.md", twice)]);

    assert!(err.contains("z.md"), "{err}");
    assert!(err.contains("set twice"), "{err}");
}

#[test]
fn a_refusal_deep_in_a_chain_names_the_file_it_came_from_once() {
    let err = refused(&[
        ("a.md", "---\nextends: y\n---\n"),
        ("y.md", "---\nextends: z\n---\n"),
        ("z.md", "---\nextends: kelpie:pm\nbody: bogus\n---\n"),
    ]);

    assert!(err.contains("z.md"), "{err}");
    assert!(!err.contains("y.md"), "{err}");
}

#[test]
fn an_unknown_key_is_refused_with_the_file() {
    for key in ["tools", "needs", "kinds", "description"] {
        let text = format!("---\nrole: pm\n{key}: x\n---\nBody.\n");

        let err = refused(&[("mine.md", &text)]);

        assert!(err.contains("mine.md"), "{err}");
        assert!(err.contains(&format!("`{key}`")), "{err}");
    }
}

#[test]
fn a_role_other_than_pm_is_refused_with_the_file() {
    let err = refused(&[("mine.md", "---\nrole: implementer\n---\nBody.\n")]);

    assert!(err.contains("mine.md"), "{err}");
    assert!(err.contains("`role`"), "{err}");
}

#[test]
fn a_file_with_no_frontmatter_is_refused_with_the_file() {
    let err = refused(&[("mine.md", "Just a body.\n")]);

    assert!(err.contains("mine.md"), "{err}");
    assert!(err.contains("frontmatter"), "{err}");
}

#[test]
fn a_blank_model_is_refused_with_its_line() {
    let err = refused(&[("mine.md", "---\nrole: pm\nmodel: \"\"\n---\n")]);

    assert!(err.contains("mine.md"), "{err}");
    assert!(err.contains("`model`"), "{err}");
    assert!(err.ends_with("at line 3"), "{err}");
}

#[test]
fn an_underscore_file_is_ignored() {
    let broken = "---\nthis: is not an agent\n---\n";
    let agents = load(&[
        ("_draft.md", broken),
        ("_pm.md", PARENT),
        ("pm.md", "---\nextends: kelpie:pm\nmodel: mine\n---\n"),
    ]);

    assert_eq!(agents.get("_draft"), None);
    assert_eq!(agents.get("_pm"), None);
    assert_eq!(agents.problems().count(), 0);
    let ignored: Vec<_> = agents
        .ignored()
        .iter()
        .map(|path| path.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(ignored, ["_draft.md", "_pm.md"]);
    assert_eq!(model(agents.get("pm").unwrap()), Some("mine"));
}

#[test]
fn a_file_that_is_no_agents_name_is_skipped() {
    let agents = load(&[("Notes.md", PARENT), ("readme.txt", "x")]);

    assert_eq!(agents.skipped().len(), 1);
    assert_eq!(agents.problems().count(), 0);
}

#[test]
fn every_resolved_key_keeps_the_layer_it_came_from() {
    let agents = load(&[
        (
            "mid.md",
            "---\nextends: kelpie:pm\nmodel: m2\neffort: low\n---\n",
        ),
        (
            "leaf.md",
            "---\nextends: mid\nbody: append\neffort: high\n---\nLeaf.\n",
        ),
    ]);

    let leaf = agents.get("leaf").unwrap();

    assert_eq!(leaf.role.from, Origin::Shipped("pm".to_owned()));
    assert_eq!(leaf.model.from, Origin::File("mid".to_owned()));
    assert_eq!(leaf.effort.from, Origin::File("leaf".to_owned()));
    assert_eq!(leaf.harness.from, Origin::BuiltIn);
    assert_eq!(leaf.body.from, Origin::File("leaf".to_owned()));
    assert_eq!(leaf.model.from.to_string(), "mid");
    assert_eq!(leaf.role.from.to_string(), "kelpie:pm");
}

#[test]
fn a_body_from_a_parent_keeps_the_parents_layer() {
    let agents = load(&[("mid.md", "---\nextends: kelpie:pm\nmodel: m2\n---\n")]);

    let mid = agents.get("mid").unwrap();

    assert_eq!(mid.body.from, Origin::Shipped("pm".to_owned()));
}

fn project_settings(dir: &Dir, text: &str) -> crate::settings::Resolved {
    let file = dir.config().file(LayerFile::Project("web")).unwrap();
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, text).unwrap();
    Settings::load(&dir.config(), "web").unwrap()
}

#[test]
fn the_pm_binding_resolves_built_in_then_global_then_project() {
    let dir = Dir::new();

    // Nothing but kelpie's own pm: it binds no harness, model or effort
    let settings = project_settings(&dir, "{}");
    let binding = dir.load().pm_binding(&settings).unwrap();
    assert_eq!(binding.harness.value, None);
    assert_eq!(binding.harness.from, Origin::BuiltIn);
    assert_eq!(binding.model.from, Origin::BuiltIn);
    assert_eq!(binding.body.value, Some(shipped_body()));
    assert_eq!(binding.body.from, Origin::Shipped("pm".to_owned()));

    // The global pm.md sets what it sets, and the rest stays kelpie's
    dir.put(
        "pm.md",
        "---\nharness: goose\nmodel: global-model\neffort: low\n---\n",
    );
    let binding = dir.load().pm_binding(&settings).unwrap();
    assert_eq!(binding.harness.value, Some(Harness::Goose));
    assert_eq!(binding.harness.from, Origin::File("pm".to_owned()));
    assert_eq!(binding.model.value.as_deref(), Some("global-model"));
    assert_eq!(binding.effort.value.as_deref(), Some("low"));
    assert_eq!(binding.body.value, Some(shipped_body()));
    assert_eq!(binding.body.from, Origin::Shipped("pm".to_owned()));

    // The project's setting beats the global file for what it sets
    let settings = project_settings(
        &dir,
        r#"{"pm-model": "project-model", "pm-harness": "claude-code"}"#,
    );
    let binding = dir.load().pm_binding(&settings).unwrap();
    assert_eq!(binding.model.value.as_deref(), Some("project-model"));
    assert_eq!(binding.model.from, Origin::Project);
    assert_eq!(binding.harness.value, Some(Harness::ClaudeCode));
    assert_eq!(binding.harness.from, Origin::Project);
    assert_eq!(binding.effort.value.as_deref(), Some("low"));
    assert_eq!(binding.effort.from, Origin::File("pm".to_owned()));
}

#[test]
fn a_global_pm_file_that_extends_nothing_still_builds_on_kelpie_pm() {
    let dir = Dir::new();
    dir.put("pm.md", "---\nmodel: mine\nbody: append\n---\nAlso this.\n");
    let settings = project_settings(&dir, "{}");

    let binding = dir.load().pm_binding(&settings).unwrap();

    assert_eq!(binding.model.value.as_deref(), Some("mine"));
    assert_eq!(
        binding.body.value,
        Some(format!("{}\n\nAlso this.", shipped_body()))
    );
}

#[test]
fn a_global_pm_file_that_cannot_be_used_stops_the_binding_naming_the_file() {
    let dir = Dir::new();
    dir.put("pm.md", "---\nharness: nope\n---\n");
    let settings = project_settings(&dir, "{}");

    let err = dir.load().pm_binding(&settings).unwrap_err().to_string();

    assert!(err.contains("pm.md"), "{err}");
    assert!(err.contains("`harness`"), "{err}");
}

#[test]
fn another_file_that_cannot_be_used_does_not_stop_the_pm() {
    let dir = Dir::new();
    dir.put("mine.md", "---\nharness: nope\n---\n");
    let settings = project_settings(&dir, "{}");

    let agents = dir.load();

    assert!(agents.pm_binding(&settings).is_ok());
    assert!(agents.broken("mine").is_some());
}

#[test]
fn a_file_that_is_not_utf8_is_left_out_and_does_not_stop_the_pm() {
    let dir = Dir::new();
    dir.put("pm.md", "---\nmodel: mine\n---\n");
    fs::write(
        dir.config().path().join(FOLDER).join("notes.md"),
        b"---\nrole: pm\n---\nCaf\xe9 notes, saved as Latin-1.\n",
    )
    .unwrap();
    let settings = project_settings(&dir, "{}");

    let agents = dir.load();

    let binding = agents.pm_binding(&settings).unwrap();
    assert_eq!(binding.model.value.as_deref(), Some("mine"));
    let err = agents.broken("notes").unwrap().to_string();
    assert!(err.contains("notes.md"), "{err}");
    assert!(err.contains("UTF-8"), "{err}");
    assert_eq!(agents.problems().count(), 1);
}

#[test]
fn a_file_that_extends_one_that_is_not_utf8_is_refused_naming_why() {
    let dir = Dir::new();
    dir.put("child.md", "---\nextends: notes\n---\n");
    fs::create_dir_all(dir.config().path().join(FOLDER)).unwrap();
    fs::write(
        dir.config().path().join(FOLDER).join("notes.md"),
        b"\xff\xfe",
    )
    .unwrap();

    let agents = dir.load();

    let err = agents.broken("child").unwrap().to_string();
    assert!(err.contains("child.md"), "{err}");
    assert!(err.contains("cannot be read"), "{err}");
}

#[test]
fn a_pm_file_that_is_not_utf8_stops_the_binding_naming_the_file() {
    let dir = Dir::new();
    fs::create_dir_all(dir.config().path().join(FOLDER)).unwrap();
    fs::write(dir.config().path().join(FOLDER).join("pm.md"), b"\xff").unwrap();
    let settings = project_settings(&dir, "{}");

    let agents = dir.load();

    let err = agents.pm_binding(&settings).unwrap_err().to_string();
    assert!(err.contains("pm.md"), "{err}");
}
