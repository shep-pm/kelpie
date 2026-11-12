//! Setting the settings page's `pm` key in the global `pm.md`

use super::*;

#[test]
fn setting_a_pm_key_creates_the_global_pm_file_over_kelpie_pm() {
    let dir = Dir::new();

    Agents::set_pm_key(&dir.config(), PmKey::Model, Some("small")).unwrap();

    let pm = dir.load().pm().unwrap();
    assert_eq!(model(&pm), Some("small"));
    assert_eq!(pm.model.from, Origin::File("pm".to_owned()));
    assert_eq!(body(&pm), Some(shipped_body().as_str()));
}

#[test]
fn setting_a_pm_key_changes_that_line_and_keeps_the_rest_of_the_file() {
    let dir = Dir::new();
    let file = "---\n# my PM\nrole: pm\nmodel: old\neffort: low\nbody: append\n---\nAlso this.\n";
    dir.put("pm.md", file);

    Agents::set_pm_key(&dir.config(), PmKey::Model, Some("new")).unwrap();
    Agents::set_pm_key(&dir.config(), PmKey::Harness, Some("goose")).unwrap();

    let text = fs::read_to_string(dir.0.join(FOLDER).join("pm.md")).unwrap();
    assert!(text.contains("# my PM\n"), "{text}");
    assert!(text.contains("effort: low\n"), "{text}");
    assert!(text.ends_with("---\nAlso this.\n"), "{text}");
    let pm = dir.load().pm().unwrap();
    assert_eq!(model(&pm), Some("new"));
    assert_eq!(pm.harness.value, Some(Harness::Goose));
    assert_eq!(effort(&pm), Some("low"));
}

#[test]
fn clearing_a_pm_key_removes_its_line_so_the_layer_below_shows() {
    let dir = Dir::new();
    dir.put("pm.md", "---\nmodel: mine\neffort: low\n---\n");

    Agents::set_pm_key(&dir.config(), PmKey::Model, None).unwrap();
    Agents::set_pm_key(&dir.config(), PmKey::Harness, None).unwrap();

    let pm = dir.load().pm().unwrap();
    assert_eq!(model(&pm), None);
    assert_eq!(pm.model.from, Origin::BuiltIn);
    assert_eq!(effort(&pm), Some("low"));
}

#[test]
fn a_pm_key_the_agent_cannot_use_is_refused_and_the_file_stays_as_it_was() {
    let dir = Dir::new();
    let before = "---\nmodel: mine\n---\n";
    dir.put("pm.md", before);

    let err = Agents::set_pm_key(&dir.config(), PmKey::Harness, Some("nope")).unwrap_err();

    assert!(err.to_string().contains("harness"), "{err}");
    let text = fs::read_to_string(dir.0.join(FOLDER).join("pm.md")).unwrap();
    assert_eq!(text, before);
}

#[test]
fn a_model_with_yaml_in_it_is_kept_as_one_string() {
    let dir = Dir::new();

    Agents::set_pm_key(&dir.config(), PmKey::Model, Some("a: b # c")).unwrap();

    assert_eq!(model(&dir.load().pm().unwrap()), Some("a: b # c"));
}
