//! Editing an agent file as a form does: its own keys and body, checked before they are written
//!
//! An editor holds a file's frontmatter keys and its body apart ([`Source`]).
//! [`Agents::check`] says what agent they would make, with the `extends` chain
//! resolved, without writing anything, and [`Agents::save`] writes them only
//! when the loader accepts them: the file in place is never touched by a refused
//! save.
//!
//! Saving writes the file again from the keys it is given, so a comment in the
//! frontmatter does not survive an edit in the form. The settings page's single
//! `pm` key keeps comments instead (`Agents::set_pm_key`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::write::{replace, scalar};
use super::{Agent, Agents, AgentsError, BUILT_IN, FOLDER, PM, extends, is_name};
use crate::settings::ConfigDir;

/// How far up an `extends` chain [`Agents::chain`] follows, so a loop in files cannot hold it
const CHAIN_LIMIT: usize = 16;

/// The keys an agent file sets itself, each as text. A key left `None` is not in the file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    /// The agent it builds on: a file's name, or `kelpie:<name>`
    pub extends: Option<String>,

    /// What the agent is for
    pub role: Option<String>,

    /// The harness it runs on
    pub harness: Option<String>,

    /// The model, as the harness names it
    pub model: Option<String>,

    /// The effort, as the harness names it
    pub effort: Option<String>,

    /// `append` to put the file's body after its parent's, instead of replacing it
    pub body: Option<String>,

    /// A reviewer's globs, which limit it to diffs that touch one
    pub paths: Option<Vec<String>>,

    /// Whether a reviewer reads a second time, shown what the first found
    pub second_look: Option<bool>,
}

/// An agent file as written: its own keys and its body
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Source {
    /// The keys the file sets
    pub frontmatter: Frontmatter,

    /// The instructions under the frontmatter, trimmed
    pub body: String,
}

impl Source {
    /// The keys and body of `text`. Keys outside the editor's eight are left out, and
    /// text with no frontmatter is all body, so the loader's refusal can still be shown.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let Some((front, body)) = extends::split(text) else {
            return Self {
                frontmatter: Frontmatter::default(),
                body: text.trim().to_owned(),
            };
        };
        let mut frontmatter = Frontmatter::default();
        for entry in extends::entries(front) {
            let value = Some(value_of(&entry)).filter(|value| !value.is_empty());
            match entry.key.as_str() {
                "extends" => frontmatter.extends = value,
                "role" => frontmatter.role = value,
                "harness" => frontmatter.harness = value,
                "model" => frontmatter.model = value,
                "effort" => frontmatter.effort = value,
                "body" => frontmatter.body = value,
                "paths" => frontmatter.paths = paths_of(&entry),
                "second_look" => {
                    frontmatter.second_look = match value.as_deref() {
                        Some("true") => Some(true),
                        Some("false") => Some(false),
                        _ => None,
                    }
                }
                _ => {}
            }
        }
        Self {
            frontmatter,
            body: body.trim().to_owned(),
        }
    }

    /// The file's text: the keys that are set, then the body
    #[must_use]
    pub fn text(&self) -> String {
        let front = &self.frontmatter;
        let mut text = String::from("---\n");
        let keys = [
            ("extends", &front.extends),
            ("role", &front.role),
            ("harness", &front.harness),
            ("model", &front.model),
            ("effort", &front.effort),
            ("body", &front.body),
        ];
        for (key, value) in keys {
            let Some(value) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) else {
                continue;
            };
            let value = if key == "extends" && is_shipped_name(value) {
                value.to_owned()
            } else {
                scalar(value)
            };
            text.push_str(&format!("{key}: {value}\n"));
        }
        // A reviewer's keys follow as flow YAML, which JSON's own escapes make safe for any glob
        if let Some(paths) = &front.paths {
            let globs = serde_json::to_string(paths).unwrap_or_else(|_| "[]".to_owned());
            text.push_str(&format!("paths: {globs}\n"));
        }
        if let Some(second_look) = front.second_look {
            text.push_str(&format!("second_look: {second_look}\n"));
        }
        text.push_str("---\n");
        let body = self.body.trim();
        if !body.is_empty() {
            text.push_str(body);
            text.push('\n');
        }
        text
    }
}

/// An entry's value. A double-quoted one is read as the JSON string [`Source::text`]
/// writes, so its escapes are undone and a quote inside it does not end it.
fn value_of(entry: &extends::Entry) -> String {
    let line = entry.text.lines().next().unwrap_or("");
    let raw = line.split_once(':').map_or("", |(_, v)| v.trim());
    if raw.starts_with('"')
        && let Some(Ok(text)) = serde_json::Deserializer::from_str(raw)
            .into_iter::<String>()
            .next()
    {
        return text;
    }
    entry.value()
}

/// A `paths` entry's globs, as a block or flow list, or `None` when it is not a list of strings
fn paths_of(entry: &extends::Entry) -> Option<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct Paths {
        paths: Vec<String>,
    }
    serde_saphyr::from_str::<Paths>(&entry.text)
        .ok()
        .map(|found| found.paths)
}

/// Whether `value` is `kelpie:<name>`, which is a plain YAML scalar as it is
fn is_shipped_name(value: &str) -> bool {
    value.strip_prefix(extends::SHIPPED).is_some_and(is_name)
}

fn file_of(dir: &ConfigDir, name: &str) -> PathBuf {
    dir.path().join(FOLDER).join(format!("{name}.md"))
}

fn refusal(path: PathBuf, message: impl Into<String>) -> AgentsError {
    AgentsError::File {
        path,
        message: message.into(),
    }
}

impl Agents {
    /// The names of the agents the folder holds, sorted: every file that can be used and every one left out
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .agents
            .keys()
            .chain(self.broken.keys())
            .cloned()
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// The file `name` as written, `None` when there is no such file
    ///
    /// # Errors
    ///
    /// [`AgentsError::File`] when `name` is no agent's name or the file is not text,
    /// and [`AgentsError::Read`] when it cannot be read.
    pub fn source(dir: &ConfigDir, name: &str) -> Result<Option<Source>, AgentsError> {
        let file = file_of(dir, name);
        if !is_name(name) {
            return Err(refusal(file, NAME));
        }
        match fs::read_to_string(&file) {
            Ok(text) => Ok(Some(Source::parse(&text))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                Err(refusal(file, "cannot be read: it is not UTF-8 text"))
            }
            Err(error) => Err(AgentsError::Read {
                path: file,
                kind: error.kind(),
            }),
        }
    }

    /// The parents `name` builds on, nearest first, ending at one of kelpie's own agents
    ///
    /// `extends` is the file's own `extends`, as an editor holds it before it is
    /// saved. A file named for one of kelpie's own agents with no `extends` builds
    /// on that one. Parents that are files are read from `dir`.
    #[must_use]
    pub fn chain(dir: &ConfigDir, name: &str, extends: Option<&str>) -> Vec<String> {
        let shipped = |file: &str| {
            BUILT_IN
                .iter()
                .any(|(known, _)| *known == file)
                .then(|| format!("{}{file}", extends::SHIPPED))
        };
        let mut chain: Vec<String> = Vec::new();
        let mut next = extends
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| shipped(name));
        while let Some(parent) = next {
            if chain.len() >= CHAIN_LIMIT || parent == name || chain.contains(&parent) {
                break;
            }
            chain.push(parent.clone());
            if parent.starts_with(extends::SHIPPED) {
                break;
            }
            let own = Self::source(dir, &parent).ok().flatten();
            next = own
                .and_then(|source| source.frontmatter.extends)
                .or_else(|| shipped(&parent));
        }
        chain
    }

    /// The agent `name` would be if its file held `source`, checked as the loader checks a file
    ///
    /// Nothing is written.
    ///
    /// # Errors
    ///
    /// [`AgentsError::File`] naming the file and the key the loader refuses, and
    /// [`AgentsError::Read`] when the folder cannot be listed.
    pub fn check(dir: &ConfigDir, name: &str, source: &Source) -> Result<Agent, AgentsError> {
        Self::checked(dir, name, &source.text())
    }

    fn checked(dir: &ConfigDir, name: &str, text: &str) -> Result<Agent, AgentsError> {
        if !is_name(name) {
            return Err(refusal(file_of(dir, name), NAME));
        }
        let after = Self::load_replacing(dir, Some((name, text)))?;
        if let Some(error) = after.broken(name) {
            return Err(error.clone());
        }
        after
            .get(name)
            .cloned()
            .ok_or_else(|| refusal(file_of(dir, name), "cannot be used"))
    }

    /// Write `source` as the file of `name`, if the loader accepts it
    ///
    /// The file is made, with the `agents` folder, when it is not there, and is
    /// replaced by a rename otherwise. Returns the agent it now is.
    ///
    /// # Errors
    ///
    /// As [`Agents::check`], in which case nothing is written, and
    /// [`AgentsError::Write`] when the file cannot be.
    pub fn save(dir: &ConfigDir, name: &str, source: &Source) -> Result<Agent, AgentsError> {
        let text = source.text();
        let agent = Self::checked(dir, name, &text)?;
        write_text(dir, name, &text)?;
        Ok(agent)
    }

    /// Make the file of `name`, building on `extends` or, with none, a plain `pm` agent
    ///
    /// # Errors
    ///
    /// [`AgentsError::File`] when the file is there already or the loader refuses it,
    /// as [`Agents::save`] otherwise.
    pub fn create(
        dir: &ConfigDir,
        name: &str,
        extends: Option<&str>,
    ) -> Result<Agent, AgentsError> {
        let file = file_of(dir, name);
        if is_name(name) && file.exists() {
            return Err(refusal(file, format!("`{name}` is an agent already")));
        }
        let mut source = Source::default();
        match extends.map(str::trim).filter(|value| !value.is_empty()) {
            Some(parent) => source.frontmatter.extends = Some(parent.to_owned()),
            None => source.frontmatter.role = Some(PM.to_owned()),
        }
        Self::save(dir, name, &source)
    }
}

const NAME: &str = "an agent's name is lowercase letters, digits and `-`";

fn write_text(dir: &ConfigDir, name: &str, text: &str) -> Result<(), AgentsError> {
    let folder = dir.path().join(FOLDER);
    let failed = |path: &Path, error: &io::Error| AgentsError::Write {
        path: path.to_path_buf(),
        kind: error.kind(),
    };
    fs::create_dir_all(&folder).map_err(|error| failed(&folder, &error))?;
    let file = file_of(dir, name);
    replace(&file, text).map_err(|error| failed(&file, &error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::Origin;
    use crate::run::RunId;

    /// A config folder of its own for one test
    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("kelpie-edit-{}", RunId::new().unwrap()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn config(&self) -> ConfigDir {
            ConfigDir::at(&self.0)
        }

        fn put(&self, file: &str, text: &str) {
            let folder = self.0.join(FOLDER);
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join(file), text).unwrap();
        }

        fn read(&self, file: &str) -> String {
            fs::read_to_string(self.0.join(FOLDER).join(file)).unwrap()
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn extending(parent: &str, body: &str) -> Source {
        Source {
            frontmatter: Frontmatter {
                extends: Some(parent.to_owned()),
                ..Frontmatter::default()
            },
            body: body.to_owned(),
        }
    }

    #[test]
    fn a_files_own_keys_and_body_are_read_apart() {
        let text = "---\nextends: kelpie:pm\n# a note\nmodel: \"opus\"\nbody: append\ncolour: red\n---\n\nMine.\n";

        let source = Source::parse(text);

        assert_eq!(source.frontmatter.extends.as_deref(), Some("kelpie:pm"));
        assert_eq!(source.frontmatter.model.as_deref(), Some("opus"));
        assert_eq!(source.frontmatter.body.as_deref(), Some("append"));
        assert_eq!(source.frontmatter.role, None);
        assert_eq!(source.body, "Mine.");
    }

    #[test]
    fn saving_a_reviewer_keeps_its_paths_and_second_look() {
        let dir = Dir::new();
        let text = "---\nextends: kelpie:reviewer\n# a note\npaths:\n  - \"src/**/*.rs\" # rust\n  - docs/*.md\nsecond_look: false\n---\nRead it.\n";
        dir.put("rust-eyes.md", text);

        let source = Agents::source(&dir.config(), "rust-eyes").unwrap().unwrap();
        assert_eq!(
            source.frontmatter.paths.as_deref(),
            Some(&["src/**/*.rs".to_owned(), "docs/*.md".to_owned()][..])
        );
        assert_eq!(source.frontmatter.second_look, Some(false));

        let agent = Agents::save(&dir.config(), "rust-eyes", &source).unwrap();

        assert_eq!(agent.paths.value, ["src/**/*.rs", "docs/*.md"]);
        assert!(!agent.second_look.value);
        let saved = Source::parse(&dir.read("rust-eyes.md"));
        assert_eq!(saved, source);
        assert!(dir.read("rust-eyes.md").contains("second_look: false\n"));
    }

    #[test]
    fn a_glob_with_yaml_in_it_is_kept_as_one_string() {
        let mut source = extending("kelpie:reviewer", "");
        source.frontmatter.paths = Some(vec![
            "a: b # c".to_owned(),
            "{x,y}/*".to_owned(),
            "yes".to_owned(),
        ]);

        assert_eq!(Source::parse(&source.text()), source);
    }

    #[test]
    fn written_keys_read_back_the_same() {
        let mut source = extending("kelpie:pm", "Line one.\n\nLine two.");
        source.frontmatter.model = Some("claude-opus-5-5".to_owned());
        source.frontmatter.effort = Some("on".to_owned());
        source.frontmatter.body = Some("append".to_owned());

        let text = source.text();

        assert!(text.starts_with("---\nextends: kelpie:pm\n"), "{text}");
        assert!(
            text.contains("effort: \"on\"\n"),
            "a word YAML reads as a boolean is quoted"
        );
        assert_eq!(Source::parse(&text), source);
    }

    #[test]
    fn a_value_with_a_backslash_or_a_quote_is_the_same_after_every_save() {
        let mut source = extending("kelpie:pm", "Body.");
        source.frontmatter.model = Some(r#"a\b"c"#.to_owned());

        let once = Source::parse(&source.text());
        let twice = Source::parse(&once.text());

        assert_eq!(once, source);
        assert_eq!(twice, source);
    }

    #[test]
    fn a_save_the_loader_accepts_is_written_and_resolves_over_its_parent() {
        let dir = Dir::new();
        let mut source = extending("kelpie:pm", "Always answer in French.");
        source.frontmatter.body = Some("append".to_owned());

        let agent = Agents::save(&dir.config(), PM, &source).unwrap();

        let body = agent.body.value.unwrap();
        assert!(body.starts_with("You are the PM"), "{body}");
        assert!(body.ends_with("Always answer in French."), "{body}");
        assert_eq!(agent.body.from, Origin::File(PM.to_owned()));
        assert_eq!(dir.read("pm.md"), source.text());
        assert!(!dir.0.join(FOLDER).join("pm.md.tmp").exists());
    }

    #[test]
    fn a_check_writes_nothing() {
        let dir = Dir::new();

        let agent = Agents::check(&dir.config(), "mine", &extending("kelpie:pm", "Hi.")).unwrap();

        assert_eq!(agent.body.value.as_deref(), Some("Hi."));
        assert!(!dir.0.join(FOLDER).exists());
    }

    #[test]
    fn a_new_agent_extends_what_it_is_given_or_is_a_plain_pm() {
        let dir = Dir::new();

        Agents::create(&dir.config(), "one", Some("kelpie:pm")).unwrap();
        Agents::create(&dir.config(), "two", None).unwrap();
        let again = Agents::create(&dir.config(), "one", None)
            .unwrap_err()
            .to_string();
        let bad = Agents::create(&dir.config(), "three", Some("nothing"))
            .unwrap_err()
            .to_string();

        assert_eq!(dir.read("one.md"), "---\nextends: kelpie:pm\n---\n");
        assert_eq!(dir.read("two.md"), "---\nrole: pm\n---\n");
        assert!(again.contains("already"), "{again}");
        assert!(bad.contains("extends"), "{bad}");
        assert!(!dir.0.join(FOLDER).join("three.md").exists());
    }

    #[test]
    fn the_chain_runs_from_the_nearest_parent_to_one_of_kelpies_own() {
        let dir = Dir::new();
        dir.put("a.md", "---\nextends: b\n---\n");
        dir.put("b.md", "---\nextends: kelpie:pm\n---\n");
        dir.put("loop-one.md", "---\nextends: loop-two\n---\n");
        dir.put("loop-two.md", "---\nextends: loop-one\n---\n");
        let config = dir.config();

        assert_eq!(Agents::chain(&config, "a", Some("b")), ["b", "kelpie:pm"]);
        assert_eq!(Agents::chain(&config, PM, None), ["kelpie:pm"]);
        assert_eq!(Agents::chain(&config, "plain", None), Vec::<String>::new());
        assert_eq!(
            Agents::chain(&config, "loop-one", Some("loop-two")),
            ["loop-two"]
        );
    }
}
