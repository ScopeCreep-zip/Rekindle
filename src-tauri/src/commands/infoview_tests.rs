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
