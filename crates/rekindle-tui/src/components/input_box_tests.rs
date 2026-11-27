use super::*;

#[test]
fn new_is_empty() {
    let input = InputBox::new();
    assert!(input.content().is_empty());
    assert_eq!(input.content_len(), 0);
    assert!(!input.is_over_limit());
}

#[test]
fn default_mode_is_compose() {
    let input = InputBox::new();
    assert_eq!(*input.mode(), InputMode::Compose);
}

#[test]
fn set_mode_reply() {
    let mut input = InputBox::new();
    input.set_mode(InputMode::Reply {
        message_id: "msg-1".into(),
        author: "alice".into(),
    });
    assert!(matches!(input.mode(), InputMode::Reply { .. }));
}

#[test]
fn clear_resets_to_compose() {
    let mut input = InputBox::new();
    input.set_mode(InputMode::Edit {
        message_id: "msg-1".into(),
    });
    input.clear();
    assert_eq!(*input.mode(), InputMode::Compose);
}

#[test]
fn title_shows_length_when_composing() {
    let input = InputBox::new();
    let title = input.title();
    assert!(title.contains("Compose"));
}

#[test]
fn title_shows_reply_target() {
    let mut input = InputBox::new();
    input.set_mode(InputMode::Reply {
        message_id: "msg-1".into(),
        author: "alice".into(),
    });
    assert!(input.title().contains("alice"));
}

#[test]
fn starts_unfocused() {
    let input = InputBox::new();
    assert!(!input.is_focused);
}

#[test]
fn focus_set_and_get() {
    let mut input = InputBox::new();
    input.set_focused(true);
    assert!(input.is_focused);
    input.set_focused(false);
    assert!(!input.is_focused);
}
