//! The window's theme, from the `theme` setting
//!
//! `System` leaves the window with the OS, which is all kelpie did before the
//! field existed. `Light` and `Dark` set the window's own theme, so the title
//! bar matches the page (`docs/design/settings.md`, Fields).
//!
//! The webview follows its window: `prefers-color-scheme` reads the theme the
//! window has, so `src/main.tsx` needs nothing of its own. `tauri.conf.json`
//! starts the window hidden, and `lib.rs` themes it before showing it, so a
//! launch paints once with the theme already on it.

use kelpie_engine::settings::Theme;
use tauri::{AppHandle, Manager as _, Runtime, Theme as WindowTheme, WebviewWindow};

/// The label of the one window kelpie opens from `tauri.conf.json`
pub const MAIN: &str = "main";

/// The theme a window is given for `theme`: `None` leaves it with the OS
#[must_use]
pub fn window_theme(theme: Theme) -> Option<WindowTheme> {
    match theme {
        Theme::System => None,
        Theme::Light => Some(WindowTheme::Light),
        Theme::Dark => Some(WindowTheme::Dark),
        // A theme kelpie has no look for yet follows the OS, as the default does
        _ => None,
    }
}

/// Set `theme` on `window`
///
/// # Errors
///
/// The window could not be themed.
pub fn apply<R: Runtime>(window: &WebviewWindow<R>, theme: Theme) -> Result<(), String> {
    window
        .set_theme(window_theme(theme))
        .map_err(|error| error.to_string())
}

/// Set `theme` on the window named [`MAIN`]
///
/// # Errors
///
/// kelpie has no window named `main`, or it could not be themed.
pub fn apply_to_main<R: Runtime>(app: &AppHandle<R>, theme: Theme) -> Result<(), String> {
    let window = app
        .get_webview_window(MAIN)
        .ok_or_else(|| format!("kelpie has no {MAIN} window to theme"))?;
    apply(&window, theme)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::mpsc;
    use std::time::Duration;

    use kelpie_engine::login::Store;
    use kelpie_engine::run::RunId;
    use kelpie_engine::settings::{ConfigDir, Theme};
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use tauri::{App, Manager as _, WebviewUrl, WebviewWindowBuilder};

    use super::{MAIN, WindowTheme, apply, window_theme};
    use crate::settings::{self, Target};
    use crate::state::AppState;

    #[test]
    fn system_clears_the_windows_theme_and_light_or_dark_sets_it() {
        assert_eq!(window_theme(Theme::System), None);
        assert_eq!(window_theme(Theme::Light), Some(WindowTheme::Light));
        assert_eq!(window_theme(Theme::Dark), Some(WindowTheme::Dark));

        let app = crate::builder_with(mock_builder(), Store::memory(), None)
            .build(mock_context(noop_assets()))
            .unwrap();
        let window = WebviewWindowBuilder::new(&app, MAIN, WebviewUrl::default())
            .build()
            .unwrap();
        for theme in [Theme::System, Theme::Light, Theme::Dark] {
            apply(&window, theme).unwrap();
        }
    }

    /// An app on the mock runtime with a project picked, its settings in `dir/config`
    fn app_with_project(dir: &std::path::Path) -> App<tauri::test::MockRuntime> {
        let state = AppState::open(
            dir.to_path_buf(),
            ConfigDir::at(dir.join("config")),
            Store::memory(),
            Box::new(|_| Err("never built".to_owned())),
            Box::new(|| Ok(None)),
            None,
        )
        .unwrap();
        state.set_project(dir.to_path_buf()).unwrap();
        mock_builder()
            .manage(state)
            .build(mock_context(noop_assets()))
            .unwrap()
    }

    /// The keys one tab's fields hold, in the order the form shows them
    fn keys(fields: &[settings::FieldView]) -> Vec<&str> {
        fields.iter().map(|field| field.key.as_str()).collect()
    }

    #[test]
    fn setting_the_theme_applies_it_to_the_main_window() {
        let dir = std::env::temp_dir().join(format!("kelpie-theme-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        let app = app_with_project(&dir);
        let state = app.state::<AppState>();
        let handle = app.handle().clone();
        let held = || fs::read_to_string(dir.join("config/settings.json")).unwrap();

        // With no window to theme, the write says so and names the window
        let error = settings::settings_set(
            handle.clone(),
            state.clone(),
            Target::Global,
            "theme".into(),
            "dark".into(),
        )
        .expect_err("kelpie has no main window yet");
        assert!(error.contains(MAIN), "{error}");
        assert!(held().contains(r#""theme": "dark""#), "{}", held());

        // With one, the same write lands and themes it
        let window = WebviewWindowBuilder::new(&app, MAIN, WebviewUrl::default())
            .build()
            .unwrap();
        assert_eq!(window.label(), MAIN);
        settings::settings_set(
            handle.clone(),
            state.clone(),
            Target::Global,
            "theme".into(),
            "light".into(),
        )
        .unwrap();
        assert!(held().contains(r#""theme": "light""#), "{}", held());

        // Resetting it hands the window back to the OS
        settings::settings_reset(handle, state, Target::Global, "theme".into()).unwrap();
        assert!(!held().contains("theme"), "{}", held());
        fs::remove_dir_all(dir).unwrap();
    }

    /// How long a theme write held behind another one is given to land anyway
    ///
    /// The write is only ever late here: the lock is held for the whole wait.
    const HELD: Duration = Duration::from_millis(200);

    /// The theme the global layer file holds, or `nothing` when it holds none
    fn held_at(dir: &std::path::Path) -> String {
        let Ok(text) = fs::read_to_string(dir.join("config/settings.json")) else {
            return "nothing".to_owned();
        };
        serde_json::from_str::<serde_json::Value>(&text).unwrap()["theme"]
            .as_str()
            .unwrap_or("nothing")
            .to_owned()
    }

    #[test]
    fn a_theme_write_waits_for_the_one_before_it_to_theme_the_window() {
        let dir = std::env::temp_dir().join(format!("kelpie-theme-lock-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        let app = app_with_project(&dir);
        let window = WebviewWindowBuilder::new(&app, MAIN, WebviewUrl::default())
            .build()
            .unwrap();
        assert_eq!(window.label(), MAIN);
        let handle = app.handle().clone();

        // A theme write holds its lock from the save through the window change, so a
        // second pick cannot save over the first and then let the first theme the
        // window on the theme the file has already moved past.
        let held = settings::THEME_WRITES.lock().unwrap();
        let (wrote, came_back) = mpsc::channel();
        std::thread::spawn(move || {
            let done = settings::settings_set(
                handle.clone(),
                handle.state::<AppState>(),
                Target::Global,
                "theme".into(),
                "dark".into(),
            );
            let _ = wrote.send(done);
        });

        std::thread::sleep(HELD);
        assert_eq!(
            held_at(&dir),
            "nothing",
            "the save ran while another theme write held the lock"
        );
        assert!(
            came_back.recv_timeout(HELD).is_err(),
            "the write came back while another theme write held the lock"
        );

        drop(held);
        came_back.recv().unwrap().unwrap();
        assert_eq!(held_at(&dir), "dark");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_theme_is_a_row_on_the_global_tab_and_nowhere_else() {
        let dir = std::env::temp_dir().join(format!("kelpie-theme-tab-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        let app = app_with_project(&dir);
        let seen = settings::settings_get(app.state::<AppState>(), None).unwrap();
        let global = keys(&seen.global);
        assert!(global.contains(&"theme"), "{global:?}");

        // What the project tab holds is what its layer may set: the machine's
        // fields are not there to be overridden
        let project_view = seen.project.unwrap();
        let project = keys(&project_view.fields);
        assert!(!project.contains(&"theme"), "{project:?}");
        assert!(!project.contains(&"bridge-updates"), "{project:?}");
        assert!(project.contains(&"read-anywhere"), "{project:?}");
        fs::remove_dir_all(dir).unwrap();
    }
}
