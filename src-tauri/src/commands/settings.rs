use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::passphrase::Passphrase;
use rekindle_types::preferences::Preferences;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_plugin_autostart::ManagerExt as _;

use crate::host_call;
use crate::state::SharedState;

/// The preferences as the settings window edits them: the identity's
/// stored preferences plus launch-at-login, which is the OS login item's
/// state (read back from the autostart plugin, never stored).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesDto {
    #[serde(flatten)]
    pub prefs: Preferences,
    pub auto_start: bool,
}

/// The unlocked identity's preferences and the OS login item's state. The
/// session's copy is read again from the database first: the Host writes
/// the read-receipt setting (`SetReadReceiptsEnabled`) there.
#[tauri::command]
pub async fn get_preferences(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<PreferencesDto, String> {
    let pool = state.db.current()?;
    let owner = crate::state_helpers::current_owner_key(state.inner())?;
    crate::services::preferences::load(state.inner(), &pool, &owner).await?;
    Ok(PreferencesDto {
        prefs: crate::services::preferences::current(state.inner()),
        auto_start: app.autolaunch().is_enabled().map_err(|e| e.to_string())?,
    })
}

/// Save the preferences, applying `auto_start` to the OS login items
/// first so a failed registration is reported instead of being stored as
/// if it had taken effect.
#[tauri::command]
pub async fn set_preferences(
    prefs: PreferencesDto,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    apply_auto_start(&app, prefs.auto_start)?;
    crate::services::preferences::update(state.inner(), |stored| *stored = prefs.prefs).await
}

/// Turn DM read receipts on or off (`IpcRequest::SetReadReceiptsEnabled`, a
/// trust-changing intent the Host reviews): the preferences come back.
#[tauri::command]
pub async fn set_read_receipts_enabled(
    enabled: bool,
    state: State<'_, SharedState>,
) -> Result<Done<Preferences>, CommandError> {
    host_call::call(&state, IpcRequest::SetReadReceiptsEnabled { enabled }).await
}

/// Share our game totals with friends or stop (`IpcRequest::SetShareGameTotals`,
/// plan E7.2, a trust-changing intent the Host reviews): the preferences
/// come back.
#[tauri::command]
pub async fn set_share_game_totals(
    enabled: bool,
    state: State<'_, SharedState>,
) -> Result<Done<Preferences>, CommandError> {
    host_call::call(&state, IpcRequest::SetShareGameTotals { enabled }).await
}

/// Scan for games or stop (`IpcRequest::SetGameDetection`, plan E7.2):
/// local only, taking effect at once; the preferences come back.
#[tauri::command]
pub async fn set_game_detection(
    enabled: bool,
    state: State<'_, SharedState>,
) -> Result<Done<Preferences>, CommandError> {
    host_call::call(&state, IpcRequest::SetGameDetection { enabled }).await
}

/// Show friends our current game or stop (`IpcRequest::SetShareCurrentGame`,
/// plan E7.2, a trust-changing intent the Host reviews): the preferences
/// come back.
#[tauri::command]
pub async fn set_share_current_game(
    enabled: bool,
    state: State<'_, SharedState>,
) -> Result<Done<Preferences>, CommandError> {
    host_call::call(&state, IpcRequest::SetShareCurrentGame { enabled }).await
}

/// What arming or disarming the quick wipe answers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickWipeState {
    pub armed: bool,
    #[serde(default)]
    pub shortcut: Option<String>,
}

/// Arm the quick wipe (`IpcRequest::QuickWipeArm`, plan E7.1: Protected,
/// the Host reviews it with exactly what a trigger destroys and asks for
/// the identity's passphrase). The panic shortcut is parsed, and refused
/// when this desktop cannot deliver it (`panic_shortcut::route`: a Wayland
/// session without the GlobalShortcuts portal), before the Host sees it;
/// it is registered once the Host armed it, and one the OS or the portal
/// refuses is an error, so the user never believes an inert shortcut is
/// armed.
#[tauri::command]
pub async fn quick_wipe_arm(
    duress_passphrase: Passphrase,
    shortcut: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<Done<QuickWipeState>, CommandError> {
    let refused = |message: String| CommandError::Failed {
        code: 400,
        message,
        remediation: None,
    };
    crate::shortcuts::parse_panic_chord(shortcut.as_deref()).map_err(refused)?;
    if shortcut.is_some() {
        if let rekindle_node::host::panic_shortcut::PanicRoute::Unavailable(reason) =
            crate::shortcuts::panic_route().await
        {
            return Err(refused(reason.into()));
        }
    }
    let done: Done<QuickWipeState> = host_call::call(
        &state,
        IpcRequest::QuickWipeArm {
            duress_passphrase,
            shortcut,
        },
    )
    .await?;
    crate::shortcuts::set_panic_chord(&app, state.inner(), done.value.shortcut.as_deref())
        .await
        .map_err(|reason| {
            CommandError::internal(format!(
                "the duress passphrase is armed, but {reason}; choose another shortcut"
            ))
        })?;
    Ok(done)
}

/// Disarm the quick wipe (`IpcRequest::QuickWipeDisarm`, Protected): the
/// duress passphrase forgotten and the panic shortcut released.
#[tauri::command]
pub async fn quick_wipe_disarm(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<Done<QuickWipeState>, CommandError> {
    let done: Done<QuickWipeState> = host_call::call(&state, IpcRequest::QuickWipeDisarm).await?;
    crate::shortcuts::set_panic_chord(&app, state.inner(), None)
        .await
        .map_err(CommandError::internal)?;
    Ok(done)
}

fn apply_auto_start(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let launcher = app.autolaunch();
    if launcher.is_enabled().map_err(|e| e.to_string())? == enabled {
        return Ok(());
    }
    let result = if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    };
    result.map_err(|e| format!("could not update launch-at-login: {e}"))
}

/// Check for application updates.
#[tauri::command]
pub async fn check_for_updates(app: tauri::AppHandle) -> Result<bool, String> {
    // TODO: Check for updates via tauri-plugin-updater
    let current_version = env!("CARGO_PKG_VERSION");
    tracing::info!(version = current_version, "checking for updates");

    // Notify frontend if an update is available
    // In production, this compares versions from the update server
    let has_update = false;
    if has_update {
        let event = rekindle_types::subscription_events::NotificationEvent::UpdateAvailable {
            version: "0.2.0".to_string(),
        };
        crate::event_dispatch::emit_notification(&app, event);
    }

    Ok(has_update)
}
