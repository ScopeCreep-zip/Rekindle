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
/// session without the GlobalShortcuts portal), before the Host sees it
/// ([`arm_checked`]). The shell registers it when the arming commits, on the
/// Host's `QuickWipeChanged` (`shell_session::unlocked`), and alerts when
/// the OS or the portal refuses it, so the user never believes an inert
/// shortcut is armed.
#[tauri::command]
pub async fn quick_wipe_arm(
    duress_passphrase: Passphrase,
    shortcut: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<QuickWipeState>, CommandError> {
    arm_checked(shortcut, crate::shortcuts::panic_route(), |shortcut| {
        host_call::call(
            &state,
            IpcRequest::QuickWipeArm {
                duress_passphrase,
                shortcut,
            },
        )
    })
    .await
}

/// What a refused chord tells the user besides the reason: nothing was
/// armed, the duress passphrase included, and how to arm it alone (plan
/// E7.1; the Linux finding of a chord refused on COSMIC that left the
/// duress passphrase silently unarmed).
pub(crate) const NOTHING_ARMED: &str =
    "Nothing was armed. Arm the duress passphrase without a shortcut to use it alone.";

/// Check the chord, then arm through `arm`. A chord the plugin cannot parse,
/// or one this desktop cannot deliver (`route`, awaited only when a chord is
/// given), is refused with its reason verbatim and [`NOTHING_ARMED`], and
/// `arm` never runs: nothing is half-armed. The refusal is logged without
/// the chord or any identifier.
async fn arm_checked<R, A, F>(
    shortcut: Option<String>,
    route: R,
    arm: A,
) -> Result<Done<QuickWipeState>, CommandError>
where
    R: std::future::Future<Output = rekindle_node::host::panic_shortcut::PanicRoute>,
    A: FnOnce(Option<String>) -> F,
    F: std::future::Future<Output = Result<Done<QuickWipeState>, CommandError>>,
{
    // The log names the kind of refusal only: never the chord, which is
    // the user's panic trigger.
    let refused = |kind: &str, message: String| {
        tracing::warn!(
            refusal = kind,
            "quick wipe arming refused before the Host: nothing armed"
        );
        CommandError::Failed {
            code: 400,
            message,
            remediation: Some(NOTHING_ARMED.to_owned()),
        }
    };
    crate::shortcuts::parse_panic_chord(shortcut.as_deref())
        .map_err(|reason| refused("unparsable chord", reason))?;
    if shortcut.is_some() {
        if let rekindle_node::host::panic_shortcut::PanicRoute::Unavailable(reason) = route.await {
            return Err(refused("undeliverable chord", reason.to_owned()));
        }
    }
    arm(shortcut).await
}

/// Disarm the quick wipe (`IpcRequest::QuickWipeDisarm`, Protected): the
/// duress passphrase forgotten; the shell releases the panic shortcut when
/// it commits, on the Host's `QuickWipeChanged`.
#[tauri::command]
pub async fn quick_wipe_disarm(
    state: State<'_, SharedState>,
) -> Result<Done<QuickWipeState>, CommandError> {
    host_call::call(&state, IpcRequest::QuickWipeDisarm).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_node::host::panic_shortcut::{PanicRoute, UNAVAILABLE};
    use std::sync::atomic::{AtomicBool, Ordering};

    async fn arm_with(
        shortcut: Option<&str>,
        route: PanicRoute,
    ) -> (Result<Done<QuickWipeState>, CommandError>, bool) {
        let armed = AtomicBool::new(false);
        let answer = arm_checked(shortcut.map(str::to_owned), async { route }, |shortcut| {
            armed.store(true, Ordering::SeqCst);
            async move {
                Ok(Done {
                    value: QuickWipeState {
                        armed: true,
                        shortcut,
                    },
                    undo: None,
                })
            }
        })
        .await;
        (answer, armed.load(Ordering::SeqCst))
    }

    /// A chord this desktop cannot deliver (COSMIC without the portal) is
    /// refused with the Host's reason verbatim and says nothing was armed;
    /// the arming never reaches the Host, so the duress passphrase is not
    /// half-armed.
    #[tokio::test]
    async fn an_undeliverable_chord_arms_nothing_and_says_why() {
        let (answer, armed) = arm_with(
            Some("CommandOrControl+Shift+F12"),
            PanicRoute::Unavailable(UNAVAILABLE),
        )
        .await;
        assert!(!armed, "the Host was asked to arm");
        assert_eq!(
            answer.expect_err("refused"),
            CommandError::Failed {
                code: 400,
                message: UNAVAILABLE.to_owned(),
                remediation: Some(NOTHING_ARMED.to_owned()),
            }
        );
    }

    /// A chord that is not one is refused the same way, and arms nothing.
    #[tokio::test]
    async fn an_unparsable_chord_arms_nothing() {
        let (answer, armed) = arm_with(Some("not a chord"), PanicRoute::OsGrab).await;
        assert!(!armed);
        assert!(matches!(
            answer,
            Err(CommandError::Failed { remediation: Some(r), .. }) if r == NOTHING_ARMED
        ));
    }

    /// The duress passphrase alone is armed whatever the desktop offers
    /// (the explicit "without a shortcut" action), and a deliverable chord
    /// goes to the Host with it.
    #[tokio::test]
    async fn without_a_chord_or_with_a_deliverable_one_the_host_arms() {
        let (answer, armed) = arm_with(None, PanicRoute::Unavailable(UNAVAILABLE)).await;
        assert!(armed && answer.is_ok());
        let (answer, armed) =
            arm_with(Some("CommandOrControl+Shift+F12"), PanicRoute::Portal).await;
        assert!(armed);
        assert_eq!(
            answer.expect("armed").value.shortcut.as_deref(),
            Some("CommandOrControl+Shift+F12")
        );
    }
}
