//! `show_os_notification` — a desktop OS notification that is safe to call
//! from inside the async runtime.
//!
//! `tauri-plugin-notification` 2.3.3 (latest published) shows notifications by
//! `tauri::async_runtime::spawn`-ing an async task that then calls the
//! *blocking* `notify_rust::Notification::show()` (see its
//! `desktop.rs::imp::Notification::show`). On the Linux/BSD (xdg) backend that
//! bottoms out in `zbus::block_on`, which panics
//! ("Cannot start a runtime from within a runtime") because the spawned task
//! runs on a Tokio worker that already has an entered runtime context. The net
//! effect on Linux is that *every* OS notification fails silently and logs a
//! panic backtrace (Tokio catches the task panic, so the app survives).
//!
//! This command bypasses the plugin's broken `show` path:
//! - **xdg (Linux/BSD):** use notify_rust's async `show_async()`, which runs on
//!   zbus's own executor — no `block_on`, no panic.
//! - **macOS / Windows:** there `show()` does *not* start a runtime, but it is
//!   still blocking I/O, so run it on `spawn_blocking`, off the async workers.
//!
//! Per-OS app-identity setup (macOS `set_application`, Windows
//! `System.AppUserModel.ID`) mirrors the plugin so notifications keep the right
//! identity and icon. The plugin stays registered for the permission APIs
//! (`isPermissionGranted` / `requestPermission`), which do not touch the broken
//! `show` path.

/// Show a desktop notification with the given `title` and `body`.
///
/// Called from the frontend's `showSystemNotification` in place of the
/// notification plugin's `sendNotification`. Errors surface as `String` for
/// the caller to log; the frontend treats this as best-effort.
#[tauri::command]
pub async fn show_os_notification(
    app: tauri::AppHandle,
    title: String,
    body: String,
) -> Result<(), String> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // xdg/zbus async path — driven by zbus's executor, never block_on.
        let _ = &app;
        let mut notification = notify_rust::Notification::new();
        notification.summary(&title).body(&body).auto_icon();
        notification
            .show_async()
            .await
            .map_err(|e| e.to_string())?;
    }

    #[cfg(any(target_os = "macos", windows))]
    {
        // `show()` doesn't start a runtime on macOS/Windows, but it blocks on
        // the platform notification API — keep it off the async workers.
        let identifier = app.config().identifier.clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
            #[cfg(target_os = "macos")]
            {
                // Mirror the plugin: in dev, masquerade as Terminal so macOS
                // shows the notification at all (unsigned dev builds have no
                // notification entitlement of their own).
                let _ = notify_rust::set_application(if tauri::is_dev() {
                    "com.apple.Terminal"
                } else {
                    &identifier
                });
            }

            let mut notification = notify_rust::Notification::new();
            notification.summary(&title).body(&body).auto_icon();

            #[cfg(windows)]
            {
                // Set System.AppUserModel.ID only for the installed app
                // (mirrors tauri-plugin-notification's desktop impl — the dev
                // build runs from target/{debug,release} and uses the default).
                let sep = std::path::MAIN_SEPARATOR;
                if let Ok(exe) = tauri::utils::platform::current_exe() {
                    if let Some(dir) = exe.parent() {
                        let curr = dir.display().to_string();
                        let debug = format!("{sep}target{sep}debug");
                        let release = format!("{sep}target{sep}release");
                        if !(curr.ends_with(&debug) || curr.ends_with(&release)) {
                            notification.app_id(&identifier);
                        }
                    }
                }
            }

            notification.show().map(|_| ()).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??;
    }

    Ok(())
}
