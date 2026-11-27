use super::*;

fn channel() -> Channel<OutboundEnvelope> {
    Channel::new(|_| Ok(()))
}

#[test]
fn latest_register_wins_and_destroy_removes() {
    let router = WebviewRouter::default();
    router.register("chat-1".into(), channel(), []);
    router.register("chat-1".into(), channel(), []);
    router.register("buddy-list".into(), channel(), []);
    assert_eq!(
        router.labels(),
        vec!["buddy-list".to_owned(), "chat-1".to_owned()]
    );
    router.remove("chat-1");
    assert_eq!(router.labels(), vec!["buddy-list".to_owned()]);
}

#[test]
fn audience_matches_labels_and_kinds() {
    let audience =
        Audience::labels(["chat-0123456789abcdef".to_owned()]).with_kinds(&[WindowKind::Community]);
    assert!(audience.includes("chat-0123456789abcdef"));
    assert!(!audience.includes("chat-fedcba9876543210"));
    assert!(audience.includes("community-browser"));
    assert!(!audience.includes("login"));
    assert!(Audience::post_auth().includes("settings"));
    assert!(!Audience::post_auth().includes("login"));
}
