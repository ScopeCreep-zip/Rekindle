//! The Infoview (plan E7.15, IV4; Infoview design §2.2): the calling
//! window's view of a subject, composed by the Host.

use rekindle_ipc::protocol::{InfoviewSubject, IpcRequest};
use rekindle_types::dto::infoview::InfoviewView;
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
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

/// `subject`'s Infoview for the calling window (`IpcRequest::InfoviewGet`);
/// the Host pushes it again to this window as it changes
/// (`SystemEvent::Infoview`, `infoview-event`).
#[tauri::command]
pub async fn get_infoview(
    window: tauri::WebviewWindow,
    subject: InfoviewSubject,
    state: State<'_, SharedState>,
) -> Result<Done<InfoviewView>, CommandError> {
    host_call::call(&state, request(window.label(), subject)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window in the request is the caller's label, whatever subject
    /// the page sends: a page cannot name another window.
    #[test]
    fn the_request_names_the_calling_window_only() {
        let subject = InfoviewSubject::Friend {
            public_key: "ab".repeat(32),
        };
        let IpcRequest::InfoviewGet { window, subject: s } =
            request("chat-0123456789abcdef", subject.clone())
        else {
            panic!("not InfoviewGet");
        };
        assert_eq!(window, "chat-0123456789abcdef");
        assert_eq!(s, subject);
        // The command's page-facing arguments are the subject alone.
        let source = include_str!("infoview.rs");
        let args = source
            .split("pub async fn get_infoview(")
            .nth(1)
            .and_then(|rest| rest.split(')').next())
            .unwrap();
        assert!(!args.contains("label"), "{args}");
        assert!(!args.contains(": String"), "{args}");
    }
}
