//! Our DM typing indicator, decided in the backend (plan E2.3).
//!
//! A window only reports its draft (`IpcRequest::DmTyping`): `typing` is
//! whether it holds text. The session state is
//! `rekindle_events::typing::TypingSessions` on the Host's
//! [`FriendStores`](crate::operations::friend::FriendStores): the first
//! input sends the start and arms the idle timer, input every
//! `TYPING_REFRESH_MS` sends it again (the receiver lapses an unrefreshed
//! start, `operations::typing`), and the stop goes out once the draft is
//! cleared or `TYPING_IDLE_MS` passes without input. Sending the message
//! ends the session without a stop frame: the receiver clears the
//! indicator when the message arrives.

use std::time::Duration;

use rekindle_codec::message::envelope::MessagePayload;
use rekindle_events::typing::{Input, Lapse};
use rekindle_types::subscription_events::TypingContext;

use super::send::send_to_peer;
use crate::operations::friend::{state, FriendError, FriendHost};

fn context(peer: &str) -> TypingContext {
    TypingContext::Dm {
        peer_key: peer.to_string(),
    }
}

/// Send our typing state to `to` (dropped, never queued, when there is no
/// route: `send_to_peer` treats typing as ephemeral).
async fn send(host: &dyn FriendHost, to: &str, typing: bool) -> Result<(), FriendError> {
    send_to_peer(host, to, &MessagePayload::TypingIndicator { typing }).await
}

/// The window's draft to `to` changed: `typing` is whether it holds text.
///
/// # Errors
/// `to` is not a public key, or the start or stop could not be sent.
pub async fn report_draft(
    host: &dyn FriendHost,
    to: &str,
    typing: bool,
) -> Result<(), FriendError> {
    state::require_public_key(to)?;
    let ctx = context(to);
    if !typing {
        let ended = host.stores().typing_out.lock().end(&ctx);
        return if ended {
            send(host, to, false).await
        } else {
            Ok(())
        };
    }
    let now = host.clock().now_ms();
    let input = host.stores().typing_out.lock().input(ctx, now);
    match input {
        Input::Started => {
            watch_idle(host, to);
            send(host, to, true).await
        }
        Input::Refresh => send(host, to, true).await,
        Input::Quiet => Ok(()),
    }
}

/// Our message to `to` went out: it ends the typing session.
pub(super) fn message_sent(host: &dyn FriendHost, to: &str) {
    host.stores().typing_out.lock().end(&context(to));
}

/// Send the stop once the session with `to` goes idle; input moves the
/// deadline, and an ended session or a lock ends the watch.
fn watch_idle(host: &dyn FriendHost, to: &str) {
    let owned = host.owned();
    let peer = to.to_string();
    host.scope()
        .spawn_or_drop("dm typing idle stop", async move {
            let ctx = context(&peer);
            loop {
                let now = owned.clock().now_ms();
                let lapse = owned.stores().typing_out.lock().idle(&ctx, now);
                match lapse {
                    Lapse::Pending(deadline) => {
                        tokio::time::sleep(Duration::from_millis(deadline.saturating_sub(now)))
                            .await;
                    }
                    Lapse::Ended => {
                        if let Err(e) = send(owned.as_ref(), &peer, false).await {
                            tracing::debug!(to = %peer, error = %e, "typing stop not sent");
                        }
                        return;
                    }
                    Lapse::Gone => return,
                }
            }
        });
}
