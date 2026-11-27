//! The Infoview (plan E7.15, IV4; Infoview design §2.2): the calling
//! window's view of a subject, composed by the Host.

use rekindle_ipc::protocol::{InfoviewSubject, IpcRequest};
use rekindle_types::dto::infoview::InfoviewView;
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::state::SharedState;

/// The request for `label`'s window. The label is the calling webview's
/// own (lead decision (48)): the page names only the subject, so it cannot
/// have another window's view pushed to it.
fn request(label: &str, subject: InfoviewSubject) -> IpcRequest {
    IpcRequest::InfoviewGet {
        subject,
        window: label.to_owned(),
    }
}

host_command! {
    state;
    /// `subject`'s Infoview for the calling window (`IpcRequest::InfoviewGet`);
    /// the Host pushes it again to this window as it changes
    /// (`SystemEvent::Infoview`, `infoview-event`).
    pub async fn get_infoview(
        window: tauri::WebviewWindow,
        subject: InfoviewSubject,
        state: State<'_, SharedState>,
    ) -> Result<Done<InfoviewView>, CommandError> {
        request(window.label(), subject)
    }
}

#[cfg(test)]
#[path = "infoview_tests.rs"]
mod tests;
