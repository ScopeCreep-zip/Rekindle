//! The Infoview through the Host (plan E7.15, IV4; Infoview design §2.2):
//! `InfoviewGet` composes a subject's sections from what this identity
//! already holds (`compose`), and one worker folds the Host's published
//! journal (lead decision (51)) to push each open window's view again as it
//! changes, coalesced per window (`worker`). No producer calls the
//! Infoview; no logic lives in a shell.
//!
//! The shell names the window that shows a subject with `InfoviewGet`'s
//! `window`. SK2's `SkinSelectionSet` (`wt/sk2main`) records the same
//! window → subject pairs here at merge, with `SkinSelection` gaining the
//! `FriendGroup` / `Community` / `None` variants this subject set has.

mod compose;
mod worker;

use std::collections::BTreeMap;

use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::dto::infoview::InfoviewSubject;

use crate::daemon::dispatch::DaemonContext;

pub(crate) use worker::run_worker;

/// Each open window's Infoview subject, by window label.
#[derive(Default)]
pub(crate) struct OpenViews(parking_lot::Mutex<BTreeMap<String, InfoviewSubject>>);

impl OpenViews {
    fn show(&self, window: String, subject: InfoviewSubject) {
        self.0.lock().insert(window, subject);
    }

    fn open(&self) -> Vec<(String, InfoviewSubject)> {
        self.0
            .lock()
            .iter()
            .map(|(w, s)| (w.clone(), s.clone()))
            .collect()
    }

    fn close_all(&self) {
        self.0.lock().clear();
    }
}

/// The longest window label `InfoviewGet` takes (a shell's labels are
/// short: `buddy-list`, `chat-<16 hex>`, `cli`).
pub(crate) const MAX_WINDOW_LABEL: usize = 128;

/// `InfoviewGet`: `subject`'s view; `window` now shows it.
pub(crate) async fn get(
    ctx: &DaemonContext,
    subject: InfoviewSubject,
    window: String,
) -> IpcResponse {
    match compose::view(ctx, &subject).await {
        Ok(view) => {
            ctx.infoview.show(window, subject);
            IpcResponse::ok(&view)
        }
        Err(e) => e,
    }
}
