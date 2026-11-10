//! The settings page's commands
//!
//! A login goes in and never comes out: [`login_set`] keeps it, [`login_status`]
//! says only whether one is set, and [`login_clear`] forgets it. No command
//! returns a login's value, and no error text holds one (`docs/design/settings.md`).
//! Each runs off the main thread, since a locked keychain can wait on the person.
//!
//! [`harness_options`] lists what a harness offers, from the harness itself.

use std::fs;

use kelpie_engine::bridge;
use kelpie_engine::harness::claude;
use kelpie_engine::login::Store;
use kelpie_engine::session::options;
use kelpie_protocol::Options;
use tauri::State;

use crate::state::AppState;

/// The folder under the data folder a harness is asked in
const ASKS_FOLDER: &str = "asks";

/// Keep `value` as `harness`'s login, replacing any it had
///
/// # Errors
///
/// The text of why the keychain refused it, or that the value was blank.
#[tauri::command(async)]
#[specta::specta]
pub fn login_set(logins: State<'_, Store>, harness: String, value: String) -> Result<(), String> {
    logins
        .set(&harness, &value)
        .map_err(|error| error.to_string())
}

/// Whether `harness` has a login
///
/// # Errors
///
/// The text of why the keychain could not be read.
#[tauri::command(async)]
#[specta::specta]
pub fn login_status(logins: State<'_, Store>, harness: String) -> Result<bool, String> {
    logins.status(&harness).map_err(|error| error.to_string())
}

/// Forget `harness`'s login
///
/// # Errors
///
/// The text of why the keychain refused it.
#[tauri::command(async)]
#[specta::specta]
pub fn login_clear(logins: State<'_, Store>, harness: String) -> Result<(), String> {
    logins.clear(&harness).map_err(|error| error.to_string())
}

/// What `harness` offers: the models and efforts it lists for itself
///
/// Kelpie asks a harness once and keeps its answer in the data folder, so
/// opening the settings page starts nothing. `refresh` asks it again. The list
/// is the harness's own, so kelpie names no model outside the default
/// (`docs/design/settings.md`).
///
/// # Errors
///
/// The text of why kelpie could not ask: the harness has no harness in kelpie
/// yet, its bridge could not be installed, it has no login, it could not be
/// started, or it did not answer.
#[tauri::command(async)]
#[specta::specta]
pub fn harness_options(
    harness: String,
    refresh: bool,
    state: State<'_, AppState>,
    logins: State<'_, Store>,
) -> Result<Options, String> {
    // Only Claude Code has a harness slice in kelpie (#12)
    if harness != claude::LOGIN {
        return Err(format!("kelpie has no {harness} harness to ask"));
    }
    // A kept answer answers without installing a bridge or starting a harness
    if !refresh && let Some(kept) = options::kept(state.data_dir(), &harness) {
        return Ok(kept);
    }
    let data = state.data_dir();
    let bridge = bridge::ensure(data, claude::pin()).map_err(|error| error.to_string())?;
    let asked = data.join(ASKS_FOLDER).join(&harness);
    fs::create_dir_all(&asked).map_err(|error| error.to_string())?;
    let harness = claude::harness(bridge, &asked, &logins).map_err(|error| error.to_string())?;
    options::offerings(
        &state.tree(),
        &harness,
        claude::LOGIN,
        data,
        &asked,
        refresh,
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use kelpie_engine::login::Store;
    use kelpie_engine::run::RunId;

    use crate::state::AppState;
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{INVOKE_KEY, get_ipc_response, mock_builder, mock_context, noop_assets};
    use tauri::webview::InvokeRequest;
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    /// The login every call sends, which no response may repeat
    const SECRET: &str = "sk-ant-oat01-the-secret";

    /// What one command answered, as the text the webview would receive
    fn invoke(
        webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
        cmd: &str,
        body: serde_json::Value,
    ) -> Result<String, String> {
        let request = InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        match get_ipc_response(webview, request) {
            Ok(body) => Ok(body.deserialize::<serde_json::Value>().unwrap().to_string()),
            Err(error) => Err(error.to_string()),
        }
    }

    #[test]
    fn no_command_returns_a_login() {
        let app = crate::builder_with(mock_builder(), Store::memory(), None)
            .build(mock_context(noop_assets()))
            .unwrap();
        let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
            .build()
            .unwrap();
        let harness = serde_json::json!({ "harness": "claude" });
        let mut said = Vec::new();

        said.push(invoke(&webview, "login_status", harness.clone()));
        said.push(invoke(
            &webview,
            "login_set",
            serde_json::json!({ "harness": "claude", "value": SECRET }),
        ));
        said.push(invoke(&webview, "login_status", harness.clone()));
        said.push(invoke(
            &webview,
            "login_set",
            serde_json::json!({ "harness": "claude", "value": "  " }),
        ));
        said.push(invoke(&webview, "login_clear", harness.clone()));
        said.push(invoke(&webview, "login_status", harness));

        let said: Vec<String> = said
            .into_iter()
            .map(|answer| answer.unwrap_or_else(|error| error))
            .collect();
        assert_eq!(said[0], "false");
        assert_eq!(said[1], "null");
        assert_eq!(said[2], "true", "status says only that a login is set");
        assert!(said[3].contains("blank"), "{}", said[3]);
        assert_eq!(said[4], "null");
        assert_eq!(said[5], "false");
        for answer in &said {
            assert!(!answer.contains(SECRET), "{answer}");
        }
    }

    #[test]
    fn there_is_no_command_that_reads_a_login_back() {
        let app = crate::builder_with(mock_builder(), Store::memory(), None)
            .build(mock_context(noop_assets()))
            .unwrap();
        let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
            .build()
            .unwrap();

        for cmd in ["login_get", "login_read", "login_show"] {
            let answer = invoke(&webview, cmd, serde_json::json!({ "harness": "claude" }));
            assert!(answer.is_err(), "{cmd} answered {answer:?}");
        }
    }

    #[test]
    fn a_harness_kelpie_cannot_ask_is_refused_before_anything_starts() {
        let dir = std::env::temp_dir().join(format!("kelpie-settings-{}", RunId::new().unwrap()));
        let state = AppState::open(
            dir.clone(),
            Store::memory(),
            Box::new(|_| Err("never built".to_owned())),
            Box::new(|| Ok(None)),
            None,
        )
        .unwrap();
        let app = crate::builder_with(mock_builder(), Store::memory(), None)
            .manage(state)
            .build(mock_context(noop_assets()))
            .unwrap();
        let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
            .build()
            .unwrap();

        let answer = invoke(
            &webview,
            "harness_options",
            serde_json::json!({ "harness": "goose", "refresh": false }),
        );

        let error = answer.expect_err("kelpie has no goose harness to ask");
        assert!(error.contains("goose"), "{error}");
        assert!(
            !dir.join("asks").exists(),
            "the refusal named a harness kelpie never started"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    /// What kelpie keeps for one harness, with the time it was asked
    const KEPT: &str = concat!(
        r#"{"asked_at":1,"models":[{"id":"small","name":"Small","#,
        r#""description":null}],"efforts":[]}"#
    );

    /// The same answer, as the command sends it to the webview
    const OFFERED: &str = concat!(
        r#"{"models":[{"id":"small","name":"Small","#,
        r#""description":null}],"efforts":[]}"#
    );

    #[test]
    fn a_kept_answer_is_read_without_installing_or_starting_anything() {
        let dir = std::env::temp_dir().join(format!("kelpie-settings-{}", RunId::new().unwrap()));
        fs::create_dir_all(dir.join("options")).unwrap();
        fs::write(dir.join("options").join("claude.json"), KEPT).unwrap();
        let state = AppState::open(
            dir.clone(),
            Store::memory(),
            Box::new(|_| Err("never built".to_owned())),
            Box::new(|| Ok(None)),
            None,
        )
        .unwrap();
        let app = crate::builder_with(mock_builder(), Store::memory(), None)
            .manage(state)
            .build(mock_context(noop_assets()))
            .unwrap();
        let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
            .build()
            .unwrap();

        let answer = invoke(
            &webview,
            "harness_options",
            serde_json::json!({ "harness": "claude", "refresh": false }),
        )
        .unwrap();

        assert_eq!(answer, OFFERED);
        assert!(
            !dir.join("bridges").exists(),
            "the kept answer installed no bridge: {}",
            dir.display()
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
