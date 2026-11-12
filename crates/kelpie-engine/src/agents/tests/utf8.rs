//! Agent files that are not UTF-8: left out, and named when they stop a binding

use std::fs;

use super::*;

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
