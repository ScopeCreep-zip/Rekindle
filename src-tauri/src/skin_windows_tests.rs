use rekindle_types::dto::skin_signals::SkinSignalSet;
use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use super::announce;
use crate::test_support::TestHost;

/// The signal set the Host serves for `window` (its standing state), once
/// one is published, within `tenths` tenths of a second.
async fn served(
    host: &rekindle_node::host::Host,
    window: &str,
    tenths: u32,
) -> Option<SkinSignalSet> {
    let find = || {
        host.standing().into_iter().find_map(|s| match s.event {
            SubscriptionEvent::System(SystemEvent::SkinSignals { window: w, set })
                if w == window =>
            {
                Some(set)
            }
            _ => None,
        })
    };
    for _ in 0..tenths {
        if let Some(set) = find() {
            return Some(set);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    None
}

/// Plan CR1: a buddy list open when the Host starts is announced then, and
/// the Host serves its signals, focused as the window is, without waiting
/// for an OS focus event (before, it was a signal window only if one
/// reached it after login).
#[tokio::test]
async fn a_buddy_list_open_at_host_start_is_served_its_signals() {
    let t = TestHost::start().await;
    assert!(
        served(&t.host, "buddy-list", 1).await.is_none(),
        "nothing announced yet"
    );
    announce(
        &t.host,
        &[("buddy-list".to_owned(), true), ("login".to_owned(), false)],
    );
    let set = served(&t.host, "buddy-list", 100)
        .await
        .expect("the buddy list's signals are served");
    assert!(set.focused, "the focused buddy list is served as focused");
    t.shutdown().await;
}

/// The login window is not drawn from a skin tree: announcing it, focused
/// or not, gives it no signals.
#[tokio::test]
async fn the_login_window_is_served_no_signals() {
    let t = TestHost::start().await;
    announce(&t.host, &[("login".to_owned(), true)]);
    assert!(
        served(&t.host, "login", 20).await.is_none(),
        "the login window has no signals"
    );
    t.shutdown().await;
}

/// An unfocused buddy list is served too: the announcement, not a focus
/// event, makes it a signal window.
#[tokio::test]
async fn an_unfocused_buddy_list_is_served_its_signals() {
    let t = TestHost::start().await;
    announce(&t.host, &[("buddy-list".to_owned(), false)]);
    let set = served(&t.host, "buddy-list", 100)
        .await
        .expect("the buddy list's signals are served");
    assert!(
        !set.focused,
        "an unfocused buddy list is served as unfocused"
    );
    t.shutdown().await;
}
