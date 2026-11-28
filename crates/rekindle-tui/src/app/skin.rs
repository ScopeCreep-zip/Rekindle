//! The skin's palette in the terminal (plan E7.7.2, "the TUI applies the
//! palette subset from the same DTO"): at start the TUI asks the Host for
//! the skin it resolved (the identity's choice, else the device login
//! skin, else the built-in default), then follows `SkinChanged`.

use std::sync::Arc;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::skin_library::SkinApplied;
use rekindle_types::subscription_events::SubscriptionEvent;

use super::commands::report_failure;
use super::App;
use crate::action::{Action, ToastLevel};

impl App {
    /// `SkinGet` for the resolution; the answer arrives as `ApplySkin`.
    pub(crate) fn load_skin(&self) {
        let tx = self.action_tx.clone();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            match client.request_ok(IpcRequest::SkinGet { id: None }).await {
                Ok(value) => match serde_json::from_value::<SkinApplied>(value) {
                    Ok(skin) => {
                        let _ = tx.send(Action::ApplySkin(Box::new(skin)));
                    }
                    Err(e) => {
                        let _ = tx.send(Action::CommandFailed {
                            context: "skin parse".into(),
                            error: e.to_string(),
                        });
                    }
                },
                Err(e) => report_failure(&tx, "skin", e),
            }
        });
    }

    /// Show `skin`'s palette; a refused one keeps the theme shown and says
    /// so.
    pub(crate) fn apply_skin(&mut self, skin: &SkinApplied) {
        let outcome = self.theme.apply_skin(skin);
        self.refused_skin(outcome);
    }

    /// The Host's `SkinChanged`: the theme follows it, then the screen.
    pub(crate) fn reskin(&mut self, event: &SubscriptionEvent) -> Action {
        if let Some(outcome) = self.theme.on_event(event) {
            self.refused_skin(outcome);
        }
        Action::Render
    }

    fn refused_skin(&mut self, outcome: anyhow::Result<()>) {
        if let Err(e) = outcome {
            self.notifications
                .push(format!("skin not applied: {e}"), ToastLevel::Warning);
        }
    }
}
