//! The theme kelpie's window follows: the OS's appearance, or one the maintainer picks
//!
//! `System` is kelpie following the OS, which is all it did before the field
//! existed. `Light` and `Dark` say otherwise, and kelpie sets the window's own
//! theme to match, so the title bar matches the page
//! (`docs/design/settings.md`, Fields).
//!
//! The field is Global only: a machine has one appearance, and a project cannot
//! change how kelpie's window looks.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How kelpie's window looks
// wire format: a settings file holds these strings
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Theme {
    /// System
    #[default]
    System,

    /// Light
    Light,

    /// Dark
    Dark,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use serde_json::{Value, json, to_value};

    use super::Theme;
    use crate::run::RunId;
    use crate::settings::{ConfigDir, Field, Layer, LayerFile, Layered, Settings, SettingsError};

    /// A config folder of its own for one test, never written over by another
    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("kelpie-theme-{}", RunId::new().unwrap()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn config(&self) -> ConfigDir {
            ConfigDir::at(&self.0)
        }

        /// Write the global layer's file by hand, the way a person or a sync would
        fn put(&self, text: &str) {
            fs::write(self.config().file(LayerFile::Global).unwrap(), text).unwrap();
        }

        fn text(&self) -> String {
            fs::read_to_string(self.config().file(LayerFile::Global).unwrap()).unwrap()
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_theme_reads_and_writes_as_system_light_or_dark() {
        let dir = Dir::new();

        Settings::set(
            &dir.config(),
            LayerFile::Global,
            Field::Theme,
            &json!("dark"),
        )
        .unwrap();
        let written = Settings::load_global(&dir.config()).unwrap();
        assert_eq!(
            written.theme,
            Layered {
                value: Theme::Dark,
                from: Layer::Global
            }
        );
        assert!(dir.text().contains(r#""theme": "dark""#), "{}", dir.text());

        // A person's own file reads the same way their editor writes it
        dir.put(r#"{"theme": "light"}"#);
        let theirs = Settings::load_global(&dir.config()).unwrap();
        assert_eq!(
            theirs.theme,
            Layered {
                value: Theme::Light,
                from: Layer::Global
            }
        );

        // A theme kelpie does not have is refused, and the file is left as it was
        let refused = Settings::set(
            &dir.config(),
            LayerFile::Global,
            Field::Theme,
            &json!("midnight"),
        );
        let error = refused.expect_err("midnight is no theme");
        assert!(
            matches!(
                &error,
                SettingsError::ValueRefused {
                    field: Field::Theme,
                    ..
                }
            ),
            "{error}"
        );
        assert!(dir.text().contains(r#""theme": "light""#), "{error}");

        for (theme, name) in [
            (Theme::System, "system"),
            (Theme::Light, "light"),
            (Theme::Dark, "dark"),
        ] {
            assert_eq!(to_value(theme).unwrap(), json!(name), "{theme:?}");
        }
    }

    #[test]
    fn the_theme_is_global_and_a_project_layer_cannot_set_it() {
        let dir = Dir::new();

        assert_eq!(Field::Theme.layers(), &[Layer::Global]);

        let refused = Settings::set(
            &dir.config(),
            LayerFile::Project("web"),
            Field::Theme,
            &json!("dark"),
        );
        match refused {
            Err(error) => assert!(
                matches!(
                    &error,
                    SettingsError::FieldNotInLayer {
                        field: Field::Theme,
                        layer: Layer::Project
                    }
                ),
                "{error}"
            ),
            Ok(()) => panic!("a project wrote the theme"),
        }
        assert!(
            !dir.config()
                .file(LayerFile::Project("web"))
                .unwrap()
                .exists()
        );

        // A project file that names it anyway is refused when it is read
        let file = dir.config().file(LayerFile::Project("web")).unwrap();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, r#"{"theme": "dark"}"#).unwrap();
        let read = Settings::load(&dir.config(), "web").expect_err("a project set the theme");
        assert!(read.to_string().contains("theme"), "{read}");

        // The global view is the only one that has a theme at all
        let global = Settings::load_global(&dir.config()).unwrap();
        assert_eq!(global.theme.from, Layer::BuiltIn);
    }

    #[test]
    fn no_global_theme_resolves_to_system_from_kelpie() {
        let dir = Dir::new();

        let settings = Settings::load(&dir.config(), "web").unwrap();
        assert_eq!(
            settings.theme,
            Layered {
                value: Theme::System,
                from: Layer::BuiltIn
            }
        );
        assert_eq!(Theme::default(), Theme::System);

        // A theme written as null is a theme nobody set
        dir.put(r#"{"theme": null}"#);
        let cleared = Settings::load_global(&dir.config()).unwrap();
        assert_eq!(
            cleared.theme,
            Layered {
                value: Theme::System,
                from: Layer::BuiltIn
            }
        );
    }

    #[test]
    fn the_theme_row_says_what_a_form_needs_to_offer_a_reset() {
        let dir = Dir::new();
        dir.put(r#"{"theme": "dark"}"#);
        let settings = Settings::load(&dir.config(), "web").unwrap();
        assert_eq!(settings.theme.from, Layer::Global);

        Settings::reset(&dir.config(), LayerFile::Global, Field::Theme).unwrap();
        let reset = Settings::load(&dir.config(), "web").unwrap();
        assert_eq!(reset.theme.from, Layer::BuiltIn);
        assert!(!dir.text().contains("theme"), "{}", dir.text());
    }

    /// The form titles each choice by its description, so the schema carries both
    #[test]
    fn the_schema_names_each_theme_with_the_word_a_form_shows() {
        let variants = crate::settings::schema()
            .pointer("/$defs/Theme/oneOf")
            .and_then(Value::as_array)
            .expect("the schema does not name the three themes")
            .clone();
        let shown: Vec<(&str, &str)> = variants
            .iter()
            .map(|variant| {
                (
                    variant["const"].as_str().unwrap(),
                    variant["description"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [("system", "System"), ("light", "Light"), ("dark", "Dark"),]
        );
    }
}
