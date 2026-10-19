//! The one inbound 1:1 dispatch, on both hosts (plan E2.1): a `'D'`
//! datagram is opened ([`prepare`]) and its payload handled here, or handed
//! back as [`Routed::Unported`] when the step that owns it has not come.
//! The desktop handles those with its existing arms; the daemon, which has
//! none of those features, drops them.

mod handlers;
mod prepare;

use rekindle_codec::message::envelope::MessagePayload;

use crate::operations::dm;
use crate::operations::friend::FriendHost;

/// An opened inbound 1:1 message.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// The sender's identity key (hex).
    pub sender_hex: String,
    pub payload: MessagePayload,
    /// The envelope's signing time, ms since the epoch.
    pub timestamp: i64,
}

/// What became of an inbound 1:1 message.
#[derive(Debug)]
pub enum Routed {
    /// Handled here, or dropped (logged).
    Handled,
    /// A payload family whose owner step has not come: the SMPL DM family
    /// and DM video (E2.3, E2.6, E4.3.8), calls and group calls (E4.1),
    /// relay offers, envelopes and status requests and push relay (E5.2),
    /// and a channel message over a 1:1 envelope (E3.1).
    Unported(Box<Prepared>),
}

/// Handle one inbound `app_message`. `datagram` starts with the `'D'` tag;
/// anything else is dropped.
pub async fn receive(host: &dyn FriendHost, datagram: &[u8]) -> Routed {
    let Some(envelope) = dm::unwire(datagram) else {
        tracing::debug!(len = datagram.len(), "dropping untagged 1:1 message");
        return Routed::Handled;
    };
    let Some(prepared) = prepare::prepare(host, envelope).await else {
        return Routed::Handled;
    };
    handlers::handle(host, prepared).await
}

/// Handle one inbound `app_call`. Its payloads (`RelayOffer`, `DmInvite`)
/// are both unported, so a handled call has no reply.
pub async fn receive_call(host: &dyn FriendHost, datagram: &[u8]) -> Routed {
    let Some(envelope) = dm::unwire(datagram) else {
        tracing::debug!(len = datagram.len(), "dropping untagged 1:1 call");
        return Routed::Handled;
    };
    let Some(prepared) = prepare::prepare(host, envelope).await else {
        return Routed::Handled;
    };
    match prepared.payload {
        MessagePayload::RelayOffer { .. } | MessagePayload::DmInvite { .. } => {
            Routed::Unported(Box::new(prepared))
        }
        _ => {
            tracing::debug!(from = %prepared.sender_hex, "1:1 call with no call payload — no reply");
            Routed::Handled
        }
    }
}
