//! The one place kelpie-app touches a platform API directly (IR-22).

/// Report a startup failure where the person who launched kelpie can see it.
///
/// A release Windows build runs as a GUI subsystem process: it has no console
/// and its stderr handle is closed, so printing there is invisible and a panic
/// risk. Everywhere else stderr is the visible place.
pub fn failure(message: &str) {
    #[cfg(windows)]
    message_box(message);
    #[cfg(not(windows))]
    eprintln!("kelpie could not open its window: {message}");
}

/// Show `message` in a Win32 message box with no owner window.
///
/// Unsafe over a safe alternative: Tauri's dialogs come from the dialog plugin,
/// which needs a built app to hang a window off, and this path runs only when
/// the app could not be built.
#[cfg(windows)]
fn message_box(message: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::PCWSTR;

    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(std::iter::once(0)).collect() };
    let caption = wide("Kelpie");
    let body = wide(&format!("kelpie could not open its window:\n{message}"));

    // SAFETY: both vectors hold NUL-terminated UTF-16 and outlive the call.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(body.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_OK | MB_ICONERROR,
        )
    };
}
