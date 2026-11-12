//! The agent editor's commands
//!
//! Global agents only (`docs/design/settings.md`, Agent files): each is
//! `<name>.md` in the config folder's `agents` folder, and the loader of
//! `kelpie_engine::agents` is the one judge of what a file may hold. A save is
//! checked by it before anything is written, so a refused save leaves the
//! folder as it was.
//!
//! [`agent_get`] gives a file's own keys and body and the agent they resolve to,
//! with the layer each key came from. [`agent_preview`] resolves keys that are
//! not saved yet, so the editor can show the agent as a session would get it
//! before the maintainer saves.

use kelpie_engine::agents::{Agent, Agents, Frontmatter as Keys, Origin, PM, Source};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::settings::FieldView;
use crate::state::AppState;

/// The keys an agent file sets itself. A key that is `null` or blank is not in the file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Frontmatter {
    /// The agent it builds on: another file's name, or `kelpie:<name>` for one of kelpie's own
    pub extends: Option<String>,

    /// What the agent is for: `pm`, `worker` or `reviewer`. An agent that extends none needs it
    pub role: Option<String>,

    /// The harness it runs on, such as `claude-code`
    pub harness: Option<String>,

    /// The model, named as the harness names it
    pub model: Option<String>,

    /// The effort, named as the harness names it
    pub effort: Option<String>,

    /// `append` puts this file's body after its parent's, and none replaces it
    pub body: Option<String>,

    /// A reviewer's globs, which limit it to diffs that touch one. The form passes them through.
    pub paths: Option<Vec<String>>,

    /// Whether a reviewer reads a second time. The form passes it through.
    pub second_look: Option<bool>,
}

impl From<Keys> for Frontmatter {
    fn from(keys: Keys) -> Self {
        Self {
            extends: keys.extends,
            role: keys.role,
            harness: keys.harness,
            model: keys.model,
            effort: keys.effort,
            body: keys.body,
            paths: keys.paths,
            second_look: keys.second_look,
        }
    }
}

impl From<Frontmatter> for Keys {
    fn from(front: Frontmatter) -> Self {
        Self {
            extends: front.extends,
            role: front.role,
            harness: front.harness,
            model: front.model,
            effort: front.effort,
            body: front.body,
            paths: front.paths,
            second_look: front.second_look,
        }
    }
}

/// One agent in the list
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AgentSummary {
    /// The file's name without `.md`
    pub name: String,

    /// Whether the file is there. The global `pm` is listed before it is, since it builds on `kelpie:pm`.
    pub exists: bool,

    /// Why the loader cannot use the file, `None` when it can
    pub problem: Option<String>,
}

/// An agent with its `extends` chain resolved
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
pub struct ResolvedAgent {
    /// `role`, `harness`, `model`, `effort` and `body`, each with the layer it came from: `kelpie:pm`, a file's name, or `kelpie`
    pub fields: Vec<FieldView>,

    /// The parents it builds on, nearest first, ending at one of kelpie's own
    pub chain: Vec<String>,
}

/// What the editor shows for one agent
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
pub struct AgentView {
    /// The file's name without `.md`
    pub name: String,

    /// Whether the file is there
    pub exists: bool,

    /// The keys the file sets itself
    pub frontmatter: Frontmatter,

    /// The file's own instructions, before its parent's are merged in
    pub body: String,

    /// Why the loader cannot use the file, `None` when it can
    pub problem: Option<String>,

    /// The agent as a session gets it, `None` when the file cannot be used
    pub resolved: Option<ResolvedAgent>,
}

fn origin_name(origin: &Origin) -> String {
    origin.to_string()
}

fn field<T: Serialize>(key: &str, value: &T, origin: &Origin) -> FieldView {
    FieldView {
        key: key.to_owned(),
        value: serde_json::to_value(value).unwrap_or(Value::Null),
        from: origin_name(origin),
    }
}

fn resolved(agent: &Agent, chain: Vec<String>) -> ResolvedAgent {
    let role = agent.role.value.name();
    ResolvedAgent {
        fields: vec![
            field("role", &role, &agent.role.from),
            field("harness", &agent.harness.value, &agent.harness.from),
            field("model", &agent.model.value, &agent.model.from),
            field("effort", &agent.effort.value, &agent.effort.from),
            field("body", &agent.body.value, &agent.body.from),
        ],
        chain,
    }
}

/// The agents the config folder holds, and the global `pm`, which is always there to edit
///
/// # Errors
///
/// The text of why the folder cannot be listed.
#[tauri::command(async)]
#[specta::specta]
pub fn agents_list(state: State<'_, AppState>) -> Result<Vec<AgentSummary>, String> {
    let agents = Agents::load(state.config()).map_err(|error| error.to_string())?;
    let mut names = agents.names();
    if !names.iter().any(|name| name == PM) {
        names.push(PM.to_owned());
        names.sort();
    }
    Ok(names
        .into_iter()
        .map(|name| AgentSummary {
            problem: agents.broken(&name).map(ToString::to_string),
            exists: name != PM || agents.get(PM).is_some() || agents.broken(PM).is_some(),
            name,
        })
        .collect())
}

/// One agent: its own keys and body, and what they resolve to with each key's layer
///
/// A file the loader cannot use still opens, with its keys and the loader's
/// reason, so it can be fixed in the form. The global `pm` opens empty before it
/// is saved, resolved as `kelpie:pm`.
///
/// # Errors
///
/// The text of why `name` is no agent: it is not a name, or no such file is there.
#[tauri::command(async)]
#[specta::specta]
pub fn agent_get(state: State<'_, AppState>, name: String) -> Result<AgentView, String> {
    let config = state.config();
    let found = Agents::source(config, &name).map_err(|error| error.to_string())?;
    let exists = found.is_some();
    if !exists && name != PM {
        return Err(format!("kelpie has no agent named {name}"));
    }
    let source = found.unwrap_or_default();
    let agents = Agents::load(config).map_err(|error| error.to_string())?;
    let chain = Agents::chain(config, &name, source.frontmatter.extends.as_deref());
    let (resolved, problem) = match (agents.get(&name), agents.broken(&name)) {
        (Some(agent), _) => (Some(resolved(agent, chain)), None),
        (None, Some(error)) => (None, Some(error.to_string())),
        (None, None) => (
            Agents::built_in(&name).map(|agent| resolved(&agent, chain)),
            None,
        ),
    };
    Ok(AgentView {
        name,
        exists,
        frontmatter: source.frontmatter.into(),
        body: source.body,
        problem,
        resolved,
    })
}

/// The agent `frontmatter` and `body` would make as `name`, for the preview beside the editor
///
/// Nothing is written. The loader checks it as it checks a file, so what it
/// refuses here is what [`agent_save`] refuses.
///
/// # Errors
///
/// The text of what the loader refuses, naming the key.
#[tauri::command(async)]
#[specta::specta]
pub fn agent_preview(
    state: State<'_, AppState>,
    name: String,
    frontmatter: Frontmatter,
    body: String,
) -> Result<ResolvedAgent, String> {
    let config = state.config();
    let source = source_of(frontmatter, body);
    let agent = Agents::check(config, &name, &source).map_err(|error| error.to_string())?;
    let chain = Agents::chain(config, &name, source.frontmatter.extends.as_deref());
    Ok(resolved(&agent, chain))
}

/// Write `frontmatter` and `body` as the file of `name`, if the loader accepts them
///
/// The loader checks the file before anything is written, so a refusal leaves
/// the folder as it was. The file is written again from the keys given, so a
/// comment in its frontmatter does not stay. The PM's next message uses a saved `pm`.
///
/// # Errors
///
/// The text of what the loader refuses, naming the key, or why the file could
/// not be written.
#[tauri::command(async)]
#[specta::specta]
pub fn agent_save(
    state: State<'_, AppState>,
    name: String,
    frontmatter: Frontmatter,
    body: String,
) -> Result<(), String> {
    let source = source_of(frontmatter, body);
    Agents::save(state.config(), &name, &source)
        .map(drop)
        .map_err(|error| error.to_string())
}

/// Make a new agent that builds on `parent`, or with none a plain `pm` agent
///
/// # Errors
///
/// The text of why not: the name is taken or is no name, or the parent is one the loader cannot find.
#[tauri::command(async)]
#[specta::specta]
pub fn agent_new(
    state: State<'_, AppState>,
    name: String,
    parent: Option<String>,
) -> Result<(), String> {
    Agents::create(state.config(), &name, parent.as_deref())
        .map(drop)
        .map_err(|error| error.to_string())
}

fn source_of(frontmatter: Frontmatter, body: String) -> Source {
    Source {
        frontmatter: frontmatter.into(),
        body,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use kelpie_engine::login::Store;
    use kelpie_engine::run::RunId;
    use kelpie_engine::settings::ConfigDir;
    use tauri::Manager as _;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    use super::{Frontmatter, agent_get, agent_new, agent_preview, agent_save, agents_list};
    use crate::state::AppState;

    /// An app on the mock runtime, its agent files in the returned folder's `config`
    fn app() -> (tauri::App<tauri::test::MockRuntime>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kelpie-agents-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        let state = AppState::open(
            dir.clone(),
            ConfigDir::at(dir.join("config")),
            Store::memory(),
            Box::new(|_| Err("never built".to_owned())),
            Box::new(|| Ok(None)),
            None,
        )
        .unwrap();
        let app = mock_builder()
            .manage(state)
            .build(mock_context(noop_assets()))
            .unwrap();
        (app, dir)
    }

    fn extending(parent: &str) -> Frontmatter {
        Frontmatter {
            extends: Some(parent.to_owned()),
            ..Frontmatter::default()
        }
    }

    fn field(view: &super::ResolvedAgent, key: &str) -> (serde_json::Value, String) {
        let found = view.fields.iter().find(|field| field.key == key).unwrap();
        (found.value.clone(), found.from.clone())
    }

    #[test]
    fn a_save_the_loader_refuses_writes_nothing() {
        let (app, dir) = app();
        let state = app.state::<AppState>();
        let agents = dir.join("config/agents");

        // No file is made, and no folder for one
        let error = agent_save(
            state.clone(),
            "mine".into(),
            extending("kelpie:nothing"),
            "Hello.".into(),
        )
        .unwrap_err();
        assert!(error.contains("extends"), "{error}");
        assert!(!agents.exists(), "a refused save made {}", agents.display());

        // A file in place is left as it was
        let mut good = extending("kelpie:pm");
        good.model = Some("opus".into());
        agent_save(state.clone(), "mine".into(), good, "Mine.".into()).unwrap();
        let before = fs::read_to_string(agents.join("mine.md")).unwrap();
        let mut bad = extending("kelpie:pm");
        bad.harness = Some("nonsense".into());
        let error = agent_save(state.clone(), "mine".into(), bad, "Changed.".into()).unwrap_err();
        assert!(error.contains("harness"), "{error}");
        assert_eq!(fs::read_to_string(agents.join("mine.md")).unwrap(), before);
        let names: Vec<_> = fs::read_dir(&agents)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["mine.md"], "nothing else was left in the folder");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_global_pm_opens_empty_and_resolved_as_kelpies_own() {
        let (app, dir) = app();
        let state = app.state::<AppState>();

        let listed = agents_list(state.clone()).unwrap();
        let view = agent_get(state.clone(), "pm".into()).unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "pm");
        assert!(!listed[0].exists);
        assert!(!view.exists);
        assert_eq!(view.frontmatter, Frontmatter::default());
        let resolved = view.resolved.unwrap();
        assert_eq!(resolved.chain, ["kelpie:pm"]);
        assert_eq!(field(&resolved, "body").1, "kelpie:pm");
        assert!(!dir.join("config/agents").exists(), "opening wrote a file");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_key_from_the_parent_says_which_layer_it_came_from() {
        let (app, dir) = app();
        let state = app.state::<AppState>();
        agent_new(state.clone(), "base".into(), Some("kelpie:pm".into())).unwrap();
        let mut set = extending("base");
        set.model = Some("opus".into());
        agent_save(state.clone(), "child".into(), set, String::new()).unwrap();

        let view = agent_get(state.clone(), "child".into()).unwrap();

        let resolved = view.resolved.unwrap();
        assert_eq!(field(&resolved, "model"), ("opus".into(), "child".into()));
        assert_eq!(field(&resolved, "role").1, "kelpie:pm");
        assert_eq!(field(&resolved, "body").1, "kelpie:pm");
        assert_eq!(resolved.chain, ["base", "kelpie:pm"]);
        assert_eq!(view.frontmatter.model.as_deref(), Some("opus"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_preview_resolves_unsaved_keys_and_writes_nothing() {
        let (app, dir) = app();
        let state = app.state::<AppState>();
        let mut append = extending("kelpie:pm");
        append.body = Some("append".into());

        let preview = agent_preview(state.clone(), "pm".into(), append, "Mine.".into()).unwrap();

        let (body, from) = field(&preview, "body");
        let body = body.as_str().unwrap().to_owned();
        assert!(body.starts_with("You are the PM"), "{body}");
        assert!(body.ends_with("Mine."), "{body}");
        assert_eq!(from, "pm");
        assert!(!dir.join("config/agents").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_the_loader_cannot_use_opens_with_its_reason() {
        let (app, dir) = app();
        let state = app.state::<AppState>();
        fs::create_dir_all(dir.join("config/agents")).unwrap();
        fs::write(
            dir.join("config/agents/odd.md"),
            "---\nrole: pm\ncolour: red\n---\nBody.\n",
        )
        .unwrap();

        let view = agent_get(state.clone(), "odd".into()).unwrap();
        let listed = agents_list(state.clone()).unwrap();

        assert!(view.problem.unwrap().contains("colour"));
        assert!(view.resolved.is_none());
        assert_eq!(view.body, "Body.");
        let odd = listed.iter().find(|agent| agent.name == "odd").unwrap();
        assert!(odd.problem.is_some());
        let error = agent_get(state.clone(), "nobody".into()).unwrap_err();
        assert!(error.contains("nobody"), "{error}");
        let error = agent_get(state.clone(), "../x".into()).unwrap_err();
        assert!(error.contains("lowercase"), "{error}");
        fs::remove_dir_all(dir).unwrap();
    }
}
