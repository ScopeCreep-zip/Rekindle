use super::*;

fn dm(peer: &str) -> TypingContext {
    TypingContext::Dm {
        peer_key: peer.to_owned(),
    }
}

#[test]
fn a_start_lapses_after_its_ttl_without_a_refresh() {
    let mut expiry = TypingExpiry::default();
    assert!(expiry.start(dm("a"), "a".into(), 1_000, TYPING_TTL_MS));
    assert_eq!(expiry.lapse(&dm("a"), "a", 5_999), Lapse::Pending(6_000));
    assert_eq!(expiry.lapse(&dm("a"), "a", 6_000), Lapse::Ended);
    assert_eq!(expiry.lapse(&dm("a"), "a", 6_001), Lapse::Gone);
}

#[test]
fn a_refresh_moves_the_deadline_and_is_not_news() {
    let mut expiry = TypingExpiry::default();
    assert!(expiry.start(dm("a"), "a".into(), 0, TYPING_TTL_MS));
    assert!(!expiry.start(dm("a"), "a".into(), 3_000, TYPING_TTL_MS));
    assert_eq!(expiry.lapse(&dm("a"), "a", 5_000), Lapse::Pending(8_000));
}

#[test]
fn a_stop_ends_the_start_and_its_timer_finds_nothing() {
    let mut expiry = TypingExpiry::default();
    expiry.start(dm("a"), "a".into(), 0, TYPING_TTL_MS);
    assert!(expiry.stop(&dm("a"), "a"));
    assert!(!expiry.stop(&dm("a"), "a"));
    assert_eq!(expiry.lapse(&dm("a"), "a", 10_000), Lapse::Gone);
}

#[test]
fn typists_are_kept_apart_by_context_and_name() {
    let mut expiry = TypingExpiry::default();
    let channel = TypingContext::Channel {
        community: "c".into(),
        channel: "general".into(),
    };
    expiry.start(dm("a"), "a".into(), 0, TYPING_TTL_MS);
    expiry.start(channel.clone(), "a".into(), 0, 10_000);
    expiry.start(channel.clone(), "b".into(), 0, 10_000);
    assert_eq!(expiry.lapse(&dm("a"), "a", 5_000), Lapse::Ended);
    assert_eq!(expiry.lapse(&channel, "a", 5_000), Lapse::Pending(10_000));
    assert!(expiry.stop(&channel, "b"));
    expiry.clear();
    assert_eq!(expiry.lapse(&channel, "a", 0), Lapse::Gone);
}

#[test]
fn a_sender_starts_once_and_refreshes_while_input_continues() {
    let mut sessions = TypingSessions::default();
    assert_eq!(sessions.input(dm("b"), 0), Input::Started);
    assert_eq!(sessions.input(dm("b"), 1_000), Input::Quiet);
    assert_eq!(sessions.input(dm("b"), 2_999), Input::Quiet);
    assert_eq!(sessions.input(dm("b"), 3_000), Input::Refresh);
    assert_eq!(sessions.input(dm("b"), 4_000), Input::Quiet);
    assert_eq!(sessions.input(dm("b"), 6_000), Input::Refresh);
}

#[test]
fn a_sender_stops_after_the_idle_time_from_its_last_input() {
    let mut sessions = TypingSessions::default();
    sessions.input(dm("b"), 0);
    sessions.input(dm("b"), 2_000);
    assert_eq!(sessions.idle(&dm("b"), 3_000), Lapse::Pending(5_000));
    assert_eq!(sessions.idle(&dm("b"), 5_000), Lapse::Ended);
    assert_eq!(sessions.idle(&dm("b"), 5_000), Lapse::Gone);
    assert_eq!(sessions.input(dm("b"), 6_000), Input::Started);
}

#[test]
fn ending_a_session_leaves_its_idle_timer_nothing() {
    let mut sessions = TypingSessions::default();
    sessions.input(dm("b"), 0);
    assert!(sessions.end(&dm("b")));
    assert!(!sessions.end(&dm("b")));
    assert_eq!(sessions.idle(&dm("b"), 10_000), Lapse::Gone);
    sessions.input(dm("c"), 0);
    sessions.clear();
    assert_eq!(sessions.idle(&dm("c"), 0), Lapse::Gone);
}

#[test]
fn an_unsent_start_leaves_no_session_and_waits_until_taken_or_ended() {
    let mut sessions = TypingSessions::default();
    let before = sessions.last_sent(&dm("b"));
    assert_eq!(sessions.input(dm("b"), 0), Input::Started);
    sessions.unsent(dm("b"), before);
    assert_eq!(sessions.last_sent(&dm("b")), None);
    assert_eq!(sessions.idle(&dm("b"), 10_000), Lapse::Gone);
    assert_eq!(sessions.input(dm("b"), 1_000), Input::Started);
    sessions.unsent(dm("b"), None);
    assert_eq!(sessions.take_waiting(|_| true), vec![dm("b")]);
    assert!(sessions.take_waiting(|_| true).is_empty());
    sessions.unsent(dm("c"), None);
    assert!(!sessions.end(&dm("c")));
    assert!(sessions.take_waiting(|_| true).is_empty());
}

#[test]
fn an_unsent_refresh_is_due_again_at_the_next_input() {
    let mut sessions = TypingSessions::default();
    sessions.input(dm("b"), 0);
    let before = sessions.last_sent(&dm("b"));
    assert_eq!(sessions.input(dm("b"), 3_000), Input::Refresh);
    sessions.unsent(dm("b"), before);
    assert_eq!(sessions.input(dm("b"), 3_100), Input::Refresh);
}
