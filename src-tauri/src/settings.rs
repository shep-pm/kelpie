//! The settings page's commands
//!
//! A login goes in and never comes out: [`login_set`] keeps it, [`login_status`]
//! says only whether one is set, and [`login_clear`] forgets it. No command
//! returns a login's value, and no error text holds one (`docs/design/settings.md`).
//! Each runs off the main thread, since a locked keychain can wait on the person.

use kelpie_engine::login::Store;
use tauri::State;

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

#[cfg(test)]
mod tests {
    use kelpie_engine::login::Store;
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
}
