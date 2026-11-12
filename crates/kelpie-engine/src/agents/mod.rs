//! Agent files: an agent is a Markdown file that may extend another
//!
//! Each agent is `<name>.md` in the `agents` folder of the config folder
//! ([`ConfigDir`]): YAML frontmatter, then a Markdown body that is the agent's
//! instructions. `extends: <name>` builds on another file, and
//! `extends: kelpie:<name>` on one of kelpie's own, which are built into the
//! binary. The keys a file sets replace its
//! parent's, a file with no body takes its parent's, and `body: append` adds
//! to it.
//!
//! The keys are `role` (`pm`, `worker` or `reviewer`), `harness`, `model`,
//! `effort`, `extends` and `body`, and a reviewer's `paths` (a list of globs
//! that limit it to diffs touching them) and `second_look` (a bool). A file
//! with any other key is refused naming the file and the key, and so is a
//! reviewer's key on a `pm` or `worker` file. A file whose name starts with `_` is ignored, as if it
//! were not there, so a draft can sit beside the real ones.
//!
//! A value belongs to the layer that set it: [`Sourced`] carries it with an
//! [`Origin`], for an editor to say *from kelpie:pm*. The PM's [`Binding`] adds
//! the project's own `pm` setting as the last layer. Kelpie names no harness
//! and no model in its own `pm`: what it runs on is a default in the built-in
//! settings layer (ADR 0006).
//!
//! A file that cannot be used is left out and listed by [`Agents::problems`], so
//! one broken draft does not stop the PM. Asking for the agent that file defines
//! is what fails.
//!
//! # Quick start
//!
//! ```no_run
//! use kelpie_engine::agents::Agents;
//! use kelpie_engine::settings::{ConfigDir, Settings};
//!
//! let dir = ConfigDir::from_env()?;
//! let agents = Agents::load(&dir)?;
//! let settings = Settings::load(&dir, "web")?;
//! let binding = agents.pm_binding(&settings)?;
//! println!("the PM's instructions come from {}", binding.body.from);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::Deserialize;

use crate::settings::{ConfigDir, Harness, Layer, Resolved};

mod edit;
mod extends;
mod write;

pub use edit::{Frontmatter, Source};
pub use write::PmKey;

#[cfg(test)]
mod tests;

/// The folder in the config folder that holds the agent files
pub const FOLDER: &str = "agents";

/// What starts the name of an agent file that kelpie ignores, as if it were absent
const IGNORED: char = '_';

/// The PM's name: kelpie's own `kelpie:pm`, and the global `pm.md` that layers over it
pub const PM: &str = "pm";

/// The worker's name: kelpie's own `kelpie:worker`, and the global `worker.md` that layers over it
pub const WORKER: &str = "worker";

/// The reviewer's name: kelpie's own `kelpie:reviewer`, and the global `reviewer.md` that layers over it
pub const REVIEWER: &str = "reviewer";

/// Kelpie's own agents, by name: what `extends: kelpie:<name>` reaches
const BUILT_IN: [(&str, &str); 3] = [
    (PM, include_str!("builtin/pm.md")),
    (WORKER, include_str!("builtin/worker.md")),
    (REVIEWER, include_str!("builtin/reviewer.md")),
];

/// What an agent is for
// wire format: an agent file holds these strings
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Role {
    /// The maintainer's one conversation: the PM
    Pm,

    /// An agent that builds one ticket in its own worktree
    Worker,

    /// An agent that reads a worker's diff and reports findings
    Reviewer,
}

impl Role {
    /// The word an agent file writes for it
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pm => "pm",
            Self::Worker => "worker",
            Self::Reviewer => "reviewer",
        }
    }
}

/// The layer a value came from
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Origin {
    /// No agent file set it, so it is kelpie's default in the built-in settings layer
    BuiltIn,

    /// One of kelpie's own agents, as `kelpie:<name>`
    Shipped(String),

    /// An agent file of this name
    File(String),

    /// The project's own setting
    Project,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BuiltIn => f.write_str(Layer::BuiltIn.name()),
            Self::Shipped(name) => write!(f, "{}{name}", extends::SHIPPED),
            Self::File(name) => f.write_str(name),
            Self::Project => f.write_str(Layer::Project.name()),
        }
    }
}

/// A value and the layer it came from
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sourced<T> {
    /// The value
    pub value: T,

    /// The layer that set it
    pub from: Origin,
}

/// One agent, with its `extends` chain resolved
///
/// Each key says which layer of the chain it came from. A key no layer sets is
/// empty and comes from [`Origin::BuiltIn`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    /// What the agent is for
    pub role: Sourced<Role>,

    /// The harness it runs on. Empty leaves it to the settings' default.
    pub harness: Sourced<Option<Harness>>,

    /// The model, named as the harness names it. Empty leaves it to the harness.
    pub model: Sourced<Option<String>>,

    /// The effort, named as the harness names it. Empty leaves it to the harness.
    pub effort: Sourced<Option<String>>,

    /// The instructions: the file's body, with its parents' merged in. None when blank.
    pub body: Sourced<Option<String>>,

    /// A reviewer's globs: it reads only diffs that touch one. Empty for every diff, and for any agent that is no reviewer.
    pub paths: Sourced<Vec<String>>,

    /// Whether a second fresh session reads again after a reviewer's first, shown what the first found. Off for any agent that is no reviewer.
    pub second_look: Sourced<bool>,
}

/// What an agent runs as: kelpie's own, the global file of its name, then the project's setting
///
/// [`Agents::pm_binding`] and [`Agents::worker_binding`] make one.
///
/// Each value is the nearest of the three layers that sets it, with that layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// The harness it runs on
    pub harness: Sourced<Option<Harness>>,

    /// The model it runs on
    pub model: Sourced<Option<String>>,

    /// How much effort it puts in
    pub effort: Sourced<Option<String>>,

    /// The instructions, which a session takes as its system prompt
    pub body: Sourced<Option<String>>,
}

/// The agents the config folder holds
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agents {
    agents: BTreeMap<String, Agent>,
    ignored: Vec<PathBuf>,
    skipped: Vec<PathBuf>,
    broken: BTreeMap<String, AgentsError>,
}

impl Agents {
    /// One of kelpie's own agents, as `extends: kelpie:<name>` reaches it
    #[must_use]
    pub fn built_in(name: &str) -> Option<Agent> {
        let (_, text) = BUILT_IN.iter().find(|(known, _)| *known == name)?;
        let context = extends::Context {
            parent: &|name| Err(format!("`extends: {name}` is no agent of kelpie's own")),
            implicit: &|_| None,
        };
        let merged = extends::merged(Origin::Shipped(name.to_owned()), text, &context).ok()?;
        parse(&merged).ok()
    }

    /// Each `<name>.md` in the `agents` folder of `dir`
    ///
    /// A folder that is not there holds no files, and nothing is created. A
    /// `.md` file whose name starts with `_` is ignored, one whose name is not
    /// lowercase letters, digits and `-` is skipped, and one that cannot be read
    /// or used, such as a file that is not UTF-8, is left out with its error in
    /// [`Agents::problems`].
    ///
    /// # Errors
    ///
    /// [`AgentsError::Read`] naming the folder, or an entry in it the listing cannot read.
    pub fn load(dir: &ConfigDir) -> Result<Self, AgentsError> {
        Self::load_replacing(dir, None)
    }

    /// [`Agents::load`], with the file of one name read as the given text instead of from disk
    ///
    /// The text stands in for the file whether or not one is there, and nothing is written.
    fn load_replacing(
        dir: &ConfigDir,
        replacing: Option<(&str, &str)>,
    ) -> Result<Self, AgentsError> {
        let folder = dir.path().join(FOLDER);
        let mut agents = Self {
            agents: BTreeMap::new(),
            ignored: Vec::new(),
            skipped: Vec::new(),
            broken: BTreeMap::new(),
        };
        let entries: Vec<io::Result<fs::DirEntry>> = match fs::read_dir(&folder) {
            Ok(entries) => entries.collect(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(failed(folder, &e)),
        };
        let mut files = Vec::new();
        for entry in entries {
            let path = entry.map_err(|e| failed(folder.clone(), &e))?.path();
            if path.extension().is_some_and(|e| e == "md") && path.is_file() {
                match is_ignored(&path) {
                    true => agents.ignored.push(path),
                    false => files.push(path),
                }
            }
        }
        if let Some((name, _)) = replacing {
            let path = folder.join(format!("{name}.md"));
            if !files.contains(&path) {
                files.push(path);
            }
        }
        files.sort();
        agents.ignored.sort();
        // Every file is read first, since one may extend another that sorts after it.
        let mut texts = BTreeMap::new();
        let mut unreadable = BTreeSet::new();
        for path in files {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if !is_name(stem) {
                agents.skipped.push(path);
                continue;
            }
            // A file that cannot be read is one that cannot be used: it is left
            // out, so a draft saved in another encoding stops nothing else.
            let read = match replacing {
                Some((name, text)) if name == stem => Ok(text.to_owned()),
                _ => fs::read_to_string(&path),
            };
            match read {
                Ok(text) => {
                    texts.insert(stem.to_owned(), (path, text));
                }
                Err(e) => {
                    let message = format!("cannot be read: {e}");
                    unreadable.insert(stem.to_owned());
                    agents
                        .broken
                        .insert(stem.to_owned(), AgentsError::File { path, message });
                }
            }
        }
        let parent = |name: &str| match name.strip_prefix(extends::SHIPPED) {
            Some(shipped) => built_in_text(shipped).ok_or_else(|| {
                let names: Vec<&str> = BUILT_IN.iter().map(|(name, _)| *name).collect();
                format!(
                    "`extends: {name}` names no shipped agent: kelpie ships: {}",
                    names.join(", ")
                )
            }),
            None if unreadable.contains(name) => Err(format!(
                "`extends: {name}` names a file that cannot be read"
            )),
            None => texts
                .get(name)
                .map(|(_, text)| extends::ParentText::File(text))
                .ok_or_else(|| {
                    format!(
                        "`extends: {name}` names no file: write `{name}.md`, or extend one of \
                         kelpie's own agents as `{}<name>`",
                        extends::SHIPPED
                    )
                }),
        };
        // A file named for one of kelpie's own agents builds on it unless it
        // extends something else, so a global `pm.md` is a layer over `kelpie:pm`
        let implicit = |file: &str| {
            BUILT_IN
                .iter()
                .any(|(name, _)| *name == file)
                .then(|| format!("{}{file}", extends::SHIPPED))
        };
        let context = extends::Context {
            parent: &parent,
            implicit: &implicit,
        };
        for (name, (path, text)) in &texts {
            let file_error = |message| AgentsError::File {
                path: path.clone(),
                message,
            };
            let merged = match extends::merged(Origin::File(name.clone()), text, &context) {
                Ok(merged) => merged,
                Err(message) => {
                    agents.broken.insert(name.clone(), file_error(message));
                    continue;
                }
            };
            match parse(&merged) {
                Ok(agent) => {
                    agents.agents.insert(name.clone(), agent);
                }
                Err(message) => {
                    agents
                        .broken
                        .insert(name.clone(), file_error(merged.place(message)));
                }
            }
        }
        Ok(agents)
    }

    /// The agent of file `name`, if it has one that can be used
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Agent> {
        self.agents.get(name)
    }

    /// What is wrong with the file of `name`, if that left it out
    #[must_use]
    pub fn broken(&self, name: &str) -> Option<&AgentsError> {
        self.broken.get(name)
    }

    /// Every file that was left out because it cannot be used, by name
    pub fn problems(&self) -> impl Iterator<Item = &AgentsError> {
        self.broken.values()
    }

    /// The `.md` files ignored because their names start with `_`, sorted
    #[must_use]
    pub fn ignored(&self) -> &[PathBuf] {
        &self.ignored
    }

    /// The `.md` files skipped because their names are no agent's, sorted
    #[must_use]
    pub fn skipped(&self) -> &[PathBuf] {
        &self.skipped
    }

    /// The PM as the files define it: the global `pm.md` over `kelpie:pm`, or `kelpie:pm` alone
    ///
    /// # Errors
    ///
    /// [`AgentsError::File`] naming `pm.md` when it cannot be used.
    pub fn pm(&self) -> Result<Agent, AgentsError> {
        self.of_kelpie(PM)
    }

    /// The worker as the files define it: the global `worker.md` over `kelpie:worker`, or `kelpie:worker` alone
    ///
    /// # Errors
    ///
    /// [`AgentsError::File`] naming `worker.md` when it cannot be used.
    pub fn worker(&self) -> Result<Agent, AgentsError> {
        self.of_kelpie(WORKER)
    }

    /// The reviewer of file `name` with its `extends` chain resolved: `reviewer` is the global `reviewer.md` over `kelpie:reviewer`, or `kelpie:reviewer` alone
    ///
    /// # Errors
    ///
    /// [`AgentsError::File`] naming the file when it cannot be used, when no
    /// file has that name, and when its role is not `reviewer`.
    pub fn reviewer(&self, name: &str) -> Result<Agent, AgentsError> {
        let agent = if name == REVIEWER {
            self.of_kelpie(REVIEWER)?
        } else {
            let file = || PathBuf::from(format!("{name}.md"));
            if let Some(error) = self.broken.get(name) {
                return Err(error.clone());
            }
            self.agents
                .get(name)
                .cloned()
                .ok_or_else(|| AgentsError::File {
                    path: file(),
                    message: format!(
                        "no reviewer of this name: write `{name}.md` with `role: reviewer`, or use `{REVIEWER}`"
                    ),
                })?
        };
        match agent.role.value {
            Role::Reviewer => Ok(agent),
            other => Err(AgentsError::File {
                path: PathBuf::from(format!("{name}.md")),
                message: format!(
                    "is not a reviewer: its `role` is `{}`, from {}",
                    other.name(),
                    agent.role.from
                ),
            }),
        }
    }

    /// The agent of kelpie's own `name` as the files define it
    fn of_kelpie(&self, name: &str) -> Result<Agent, AgentsError> {
        if let Some(error) = self.broken.get(name) {
            return Err(error.clone());
        }
        match self.agents.get(name) {
            Some(agent) => Ok(agent.clone()),
            None => Self::built_in(name).ok_or_else(|| AgentsError::File {
                path: PathBuf::from(format!("{}{name}", extends::SHIPPED)),
                message: format!("kelpie's own {name} cannot be read"),
            }),
        }
    }

    /// What the PM runs as for a project: `kelpie:pm`, then the global `pm.md`, then the project's setting
    ///
    /// The project's harness, model and effort are the fields `settings` holds
    /// for it. Where it sets none, the agent's own stand.
    ///
    /// # Errors
    ///
    /// As [`Agents::pm`].
    pub fn pm_binding(&self, settings: &Resolved) -> Result<Binding, AgentsError> {
        let pm = self.pm()?;
        Ok(Binding {
            harness: project_over(pm.harness, &settings.pm_harness),
            model: project_over(pm.model, &settings.pm_model),
            effort: project_over(pm.effort, &settings.pm_effort),
            body: pm.body,
        })
    }

    /// What a worker runs as for a project: `kelpie:worker`, then the global `worker.md`, then the project's setting
    ///
    /// The project's harness, model and effort are the fields `settings` holds
    /// for it. Where it sets none, the agent's own stand.
    ///
    /// # Errors
    ///
    /// As [`Agents::worker`].
    pub fn worker_binding(&self, settings: &Resolved) -> Result<Binding, AgentsError> {
        let worker = self.worker()?;
        Ok(Binding {
            harness: project_over(worker.harness, &settings.worker_harness),
            model: project_over(worker.model, &settings.worker_model),
            effort: project_over(worker.effort, &settings.worker_effort),
            body: worker.body,
        })
    }
}

/// `agent`'s value, unless the project's setting sets one
fn project_over<T: Clone>(agent: Sourced<T>, project: &crate::settings::Layered<T>) -> Sourced<T> {
    match project.from {
        Layer::Project => Sourced {
            value: project.value.clone(),
            from: Origin::Project,
        },
        Layer::BuiltIn | Layer::Global => agent,
    }
}

/// The text of the agent of kelpie's own `name`
fn built_in_text(name: &str) -> Option<extends::ParentText<'static>> {
    BUILT_IN
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(name, text)| extends::ParentText::Shipped { name, text })
}

fn failed(path: PathBuf, error: &io::Error) -> AgentsError {
    AgentsError::Read {
        path,
        kind: error.kind(),
    }
}

/// Whether the file at `path` is one kelpie ignores, whose name starts with `_`
fn is_ignored(path: &std::path::Path) -> bool {
    let name = path.file_name().and_then(|name| name.to_str());
    name.is_some_and(|name| name.starts_with(IGNORED))
}

/// An agent's name is lowercase letters, digits and `-`
fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

// An agent file's frontmatter. Its `extends` and `body` keys were taken out by
// the merge, so a `body` left here is on a file that extends nothing, and is
// refused as any other unknown key is.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Front {
    role: Role,
    #[serde(default)]
    harness: Option<Harness>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    effort: Option<String>,
    // A reviewer's keys: refused by name on any other role
    #[serde(default)]
    paths: Option<Vec<String>>,
    #[serde(default)]
    second_look: Option<bool>,
}

/// The agent the merged text defines, or what is wrong with it
fn parse(merged: &extends::Merged) -> Result<Agent, String> {
    let Some((front, body)) = extends::split(&merged.text) else {
        return Err("must start with a `---` line, then the YAML frontmatter, \
                    then a `---` line of its own"
            .into());
    };
    // A blank first line stands for the opening `---`, so a message's line is the file's.
    if front.trim().is_empty() {
        return Err("`role` is not set: an agent that extends none needs `role: pm`".into());
    }
    let yaml = format!("\n{front}");
    let keys: Front = serde_saphyr::from_str(&yaml).map_err(|error| yaml_error(&yaml, &error))?;
    let sourced = |key: &str| merged.origins.get(key).cloned().unwrap_or(Origin::BuiltIn);
    let named = |key: &str, value: Option<String>| match value {
        Some(value) if value.trim().is_empty() => {
            let line =
                extends::line_of(front, key).map_or(String::new(), |n| format!(" at line {n}"));
            Err(format!(
                "`{key}` is blank: leave the key out to keep the harness's own{line}"
            ))
        }
        value => Ok(Sourced {
            value,
            from: sourced(key),
        }),
    };
    let line =
        |key: &str| extends::line_of(front, key).map_or(String::new(), |n| format!(" at line {n}"));
    if keys.role != Role::Reviewer {
        let set = [
            ("paths", keys.paths.is_some()),
            ("second_look", keys.second_look.is_some()),
        ];
        if let Some((key, _)) = set.into_iter().find(|(_, set)| *set) {
            return Err(format!(
                "`{key}` is a reviewer's key: a `{}` file takes none, so set `role: reviewer` \
                 or take it out{}",
                keys.role.name(),
                line(key)
            ));
        }
    }
    if keys
        .paths
        .as_ref()
        .is_some_and(|paths| paths.iter().any(|glob| glob.trim().is_empty()))
    {
        return Err(format!(
            "`paths` has a blank glob: take it out{}",
            line("paths")
        ));
    }
    let body = body.trim();
    Ok(Agent {
        role: Sourced {
            value: keys.role,
            from: sourced("role"),
        },
        harness: Sourced {
            value: keys.harness,
            from: sourced("harness"),
        },
        model: named("model", keys.model)?,
        effort: named("effort", keys.effort)?,
        body: Sourced {
            value: (!body.is_empty()).then(|| body.to_owned()),
            from: sourced(extends::BODY),
        },
        paths: Sourced {
            value: keys.paths.unwrap_or_default(),
            from: sourced("paths"),
        },
        second_look: Sourced {
            value: keys.second_look.unwrap_or(false),
            from: sourced("second_look"),
        },
    })
}

// A frontmatter refusal with the key it is about and the line it is on
fn yaml_error(yaml: &str, error: &serde_saphyr::Error) -> String {
    let message = error.without_snippet().to_string();
    let key = error
        .location()
        .filter(|at| at.column() > 1)
        .and_then(|at| {
            let lines: Vec<&str> = yaml.lines().collect();
            let at = usize::try_from(at.line()).ok()?.checked_sub(1)?;
            let line = lines.get(at)?;
            let (key, _) = line.split_once(':')?;
            Some(key.trim()).filter(|k| !k.is_empty() && !k.starts_with(['#', '-']))
        });
    match key {
        Some(key) => format!("`{key}`: {message}"),
        None => message,
    }
}

/// Why an agent file cannot be used
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AgentsError {
    /// The folder, or an entry in it, could not be listed
    Read {
        /// The folder or file
        path: PathBuf,
        /// What reading it failed with
        kind: io::ErrorKind,
    },

    /// A file could not be written
    Write {
        /// The file or folder
        path: PathBuf,
        /// What writing it failed with
        kind: io::ErrorKind,
    },

    /// A file has no frontmatter, a key unknown or malformed, or a parent it cannot use
    File {
        /// The file
        path: PathBuf,
        /// What is wrong, naming the key
        message: String,
    },
}

impl fmt::Display for AgentsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, kind } => write!(f, "agent files: {}: {kind}", path.display()),
            Self::Write { path, kind } => {
                write!(f, "agent files: cannot write {}: {kind}", path.display())
            }
            Self::File { path, message } => {
                write!(f, "agent file {}: {message}", path.display())
            }
        }
    }
}

impl core::error::Error for AgentsError {}
