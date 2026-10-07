//! OS notifications, decided by `rekindle_events::notify_policy` and shown
//! from Rust.
//!
//! The dispatch loop hands every candidate event to [`notify`], which
//! gathers the user's state (preferences, Do Not Disturb, quiet hours,
//! call and presence state), asks the shared policy, and shows the result.
//! One process decides once, however many windows are open.
//!
//! ## Showing a notification
//!
//! `tauri-plugin-notification` shows notifications by
//! `tauri::async_runtime::spawn`-ing a task that calls the *blocking*
//! `notify_rust::Notification::show()`. On the Linux/BSD (xdg) backend that
//! bottoms out in `zbus::block_on`, which panics ("Cannot start a runtime
//! from within a runtime") on a Tokio worker, so every OS notification
//! failed silently on Linux. [`show`] bypasses it:
//! - **xdg (Linux/BSD):** notify_rust's async `show_async()`, on zbus's own
//!   executor — no `block_on`, no panic.
//! - **macOS / Windows:** `show()` does not start a runtime there, but it is
//!   still blocking I/O, so it runs on `spawn_blocking`.
//!
//! Per-OS app identity (macOS `set_application`, Windows
//! `System.AppUserModel.ID`) mirrors the plugin so notifications keep the
//! right identity and icon.

use std::sync::Arc;

use rekindle_events::notify_policy::{self, NotifyInputs};
use rekindle_types::subscription_events::SubscriptionEvent;

use crate::state::AppState;

/// Show the OS notification `event` calls for, if any. Returns at once;
/// the inputs are read and the notification shown on a task.
pub fn notify(app: &tauri::AppHandle, state: &Arc<AppState>, event: &SubscriptionEvent) {
    if !notify_policy::is_candidate(event) {
        return;
    }
    let (app, state, event) = (app.clone(), Arc::clone(state), event.clone());
    crate::state_helpers::login_scope_or_closed(&state).spawn_or_drop(
        "os notification",
        async move {
            let inputs = gather_inputs(&app, &state).await;
            let Some(notification) = notify_policy::os_notification_for(&event, &inputs) else {
                return;
            };
            if let Err(e) = show(&app, notification.title, notification.body).await {
                tracing::warn!(error = %e, "OS notification failed");
            }
        },
    );
}

async fn gather_inputs(app: &tauri::AppHandle, state: &Arc<AppState>) -> NotifyInputs {
    use crate::services::community::notifications::{
        is_do_not_disturb_active, is_quiet_hours_active,
    };

    let prefs = crate::commands::settings::load_preferences(app);
    // Without an identity database there is no DND or quiet-hours setting
    // to honour; notifications are then decided by the global switch alone.
    let pool = state.db.current().ok();
    let call_active = state.active_calls.list_all().iter().any(|call| {
        matches!(
            call.status,
            rekindle_calls::CallStatus::Connecting | rekindle_calls::CallStatus::Active
        )
    });
    NotifyInputs {
        enabled: prefs.notifications_enabled,
        do_not_disturb: match &pool {
            Some(pool) => is_do_not_disturb_active(state, pool).await,
            None => false,
        },
        quiet_hours_active: match &pool {
            Some(pool) => is_quiet_hours_active(state, pool).await.unwrap_or(false),
            None => false,
        },
        in_call_dnd: prefs.in_call_dnd_auto_enable,
        call_active,
        status_busy: crate::state_helpers::identity_status(state)
            == Some(crate::state::UserStatus::Busy),
    }
}

async fn show(app: &tauri::AppHandle, title: String, body: String) -> Result<(), String> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // xdg/zbus async path — driven by zbus's executor, never block_on.
        let _ = app;
        let mut notification = notify_rust::Notification::new();
        notification.summary(&title).body(&body).auto_icon();
        notification.show_async().await.map_err(|e| e.to_string())?;
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
