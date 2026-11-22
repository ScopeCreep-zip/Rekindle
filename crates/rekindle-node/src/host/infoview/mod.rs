//! The Infoview through the Host (plan E7.15, IV4; Infoview design §2.2):
//! `InfoviewGet` composes a subject's sections from what this identity
//! already holds (`compose`), and one worker folds the Host's published
//! journal (lead decision (51)) to push each open window's view again as it
//! changes, coalesced per window (`worker`). No producer calls the
//! Infoview; no logic lives in a shell.
//!
//! The shell names the window that shows a subject with `InfoviewGet`'s
//! `window`. A pack's Info popup asks `Selection` (plan E7.15, IV5): its
//! page follows its launcher's selection (`SkinSelectionSet`, lead decision
//! 48; the launcher from `Host::skin_window_opened`, plan SK2-N4), as
//! Xfire's Infoview followed the buddy list, and a change of that selection
//! pushes the page again to the windows that follow it, coalesced like
//! every other push.

mod compose;
mod worker;

use std::collections::{BTreeMap, BTreeSet};

use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::dto::infoview::InfoviewSubject;
use rekindle_types::dto::skin_run::SkinSelection;

use crate::daemon::dispatch::DaemonContext;

pub(crate) use worker::run_worker;

/// Each open window's Infoview subject as it asked (`Selection` stays
/// `Selection`), by window label, and the windows whose selection changed
/// since the last push.
#[derive(Default)]
pub(crate) struct OpenViews {
    views: parking_lot::Mutex<BTreeMap<String, InfoviewSubject>>,
    selections: parking_lot::Mutex<BTreeSet<String>>,
    changed: tokio::sync::Notify,
}

impl OpenViews {
    fn show(&self, window: String, subject: InfoviewSubject) {
        self.views.lock().insert(window, subject);
    }

    fn open(&self) -> Vec<(String, InfoviewSubject)> {
        self.views
            .lock()
            .iter()
            .map(|(w, s)| (w.clone(), s.clone()))
            .collect()
    }

    /// `window` closed: nothing is pushed to it again.
    pub(crate) fn close(&self, window: &str) {
        self.views.lock().remove(window);
    }

    fn close_all(&self) {
        self.views.lock().clear();
        self.selections.lock().clear();
    }

    /// `window`'s selection changed: the views that follow it are composed
    /// again at the worker's next push.
    pub(crate) fn selection_changed(&self, window: &str) {
        self.selections.lock().insert(window.to_owned());
        self.changed.notify_one();
    }

    /// The windows whose selection changed since the last call.
    fn take_changed(&self) -> BTreeSet<String> {
        std::mem::take(&mut *self.selections.lock())
    }
}

/// The window whose selection `window`'s `Selection` follows: the popup's
/// launcher, or `window` itself when it is not a popup.
fn launcher(ctx: &DaemonContext, window: &str) -> String {
    ctx.skin
        .launchers
        .of(window)
        .unwrap_or_else(|| window.to_owned())
}

/// The subject `asked` names for `window`: `Selection` resolved from the
/// launcher's selection, every other subject as asked. Never `Selection`.
fn resolved(ctx: &DaemonContext, window: &str, asked: &InfoviewSubject) -> InfoviewSubject {
    if *asked != InfoviewSubject::Selection {
        return asked.clone();
    }
    match ctx.skin.selection.get(&launcher(ctx, window)) {
        Some(SkinSelection::Friend { public_key }) => InfoviewSubject::Friend { public_key },
        Some(SkinSelection::FriendGroup { group_id }) => InfoviewSubject::FriendGroup { group_id },
        // The subject set has no page for a server, a transfer, a media
        // file or a room (design §2.2): with none of its subjects selected
        // the page is our own, as with nothing selected.
        None
        | Some(
            SkinSelection::Server { .. }
            | SkinSelection::Transfer { .. }
            | SkinSelection::Media { .. }
            | SkinSelection::Group { .. },
        ) => InfoviewSubject::Own,
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
    match compose::view(ctx, &resolved(ctx, &window, &subject)).await {
        Ok(view) => {
            ctx.infoview.show(window, subject);
            IpcResponse::ok(&view)
        }
        Err(e) => e,
    }
}
