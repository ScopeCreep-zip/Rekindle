//! Strand relay, status-probe and push-relay bodies, moved unchanged from
//! their `MessagePayload` variants (they stay with their `src-tauri`
//! owners until E5.2 and E5.6).

use crate::capnp_codec::direct::{data, list_len, opt_text, set_text_list, text, text_list};
use crate::capnp_codec::CodecError;
use crate::direct_capnp::{
    register_push_relay, relay_envelope, relay_offer, relay_offer_ack, relay_withdraw,
    status_request, status_response, unregister_push_relay, wake_notify,
};

/// 0x0003 RelayEnvelope: forward `inner_payload` to `target_pubkey`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayEnvelope {
    pub target_pubkey: String,
    pub inner_payload: Vec<u8>,
}

impl RelayEnvelope {
    pub(super) fn write(&self, mut b: relay_envelope::Builder<'_>) {
        b.set_target_pubkey(&self.target_pubkey);
        b.set_inner_payload(&self.inner_payload);
    }

    pub(super) fn read(r: relay_envelope::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            target_pubkey: text(r.get_target_pubkey())?,
            inner_payload: data(r.get_inner_payload())?,
        })
    }
}

/// 0x0130 RelayOffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayOffer {
    pub relay_route_blob: Vec<u8>,
    pub relay_pseudonym: String,
}

impl RelayOffer {
    pub(super) fn write(&self, mut b: relay_offer::Builder<'_>) {
        b.set_relay_route_blob(&self.relay_route_blob);
        b.set_relay_pseudonym(&self.relay_pseudonym);
    }

    pub(super) fn read(r: relay_offer::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            relay_route_blob: data(r.get_relay_route_blob())?,
            relay_pseudonym: text(r.get_relay_pseudonym())?,
        })
    }
}

/// 0x0131 RelayWithdraw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayWithdraw {
    pub relay_pseudonym: String,
}

impl RelayWithdraw {
    pub(super) fn write(&self, mut b: relay_withdraw::Builder<'_>) {
        b.set_relay_pseudonym(&self.relay_pseudonym);
    }

    pub(super) fn read(r: relay_withdraw::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            relay_pseudonym: text(r.get_relay_pseudonym())?,
        })
    }
}

/// 0x0132 RelayOfferAck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayOfferAck {
    pub ok: bool,
    pub reason: String,
}

impl RelayOfferAck {
    pub(super) fn write(&self, mut b: relay_offer_ack::Builder<'_>) {
        b.set_ok(self.ok);
        b.set_reason(&self.reason);
    }

    pub(super) fn read(r: relay_offer_ack::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            ok: r.get_ok(),
            reason: text(r.get_reason())?,
        })
    }
}

/// 0x0140 StatusRequest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRequest {
    pub target_pubkey: String,
}

impl StatusRequest {
    pub(super) fn write(&self, mut b: status_request::Builder<'_>) {
        b.set_target_pubkey(&self.target_pubkey);
    }

    pub(super) fn read(r: status_request::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            target_pubkey: text(r.get_target_pubkey())?,
        })
    }
}

/// 0x0141 StatusResponse; an empty `status` means "no data for this
/// peer".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResponse {
    pub target_pubkey: String,
    pub status: String,
    pub status_message: Option<String>,
    /// Unix seconds of the last presence update seen for the peer.
    pub last_seen: u64,
    pub route_blob: Vec<u8>,
}

impl StatusResponse {
    pub(super) fn write(&self, mut b: status_response::Builder<'_>) {
        b.set_target_pubkey(&self.target_pubkey);
        b.set_status(&self.status);
        b.set_status_message(self.status_message.as_deref().unwrap_or_default());
        b.set_last_seen(self.last_seen);
        b.set_route_blob(&self.route_blob);
    }

    pub(super) fn read(r: status_response::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            target_pubkey: text(r.get_target_pubkey())?,
            status: text(r.get_status())?,
            status_message: opt_text(r.get_status_message())?,
            last_seen: r.get_last_seen(),
            route_blob: data(r.get_route_blob())?,
        })
    }
}

/// 0x0310 RegisterPushRelay (sealed to the relay's identity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterPushRelay {
    pub device_push_token: String,
    pub platform: String,
    pub record_keys: Vec<String>,
}

impl RegisterPushRelay {
    pub(super) fn write(&self, mut b: register_push_relay::Builder<'_>) {
        b.set_device_push_token(&self.device_push_token);
        b.set_platform(&self.platform);
        set_text_list(
            b.init_record_keys(list_len(&self.record_keys)),
            &self.record_keys,
        );
    }

    pub(super) fn read(r: register_push_relay::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            device_push_token: text(r.get_device_push_token())?,
            platform: text(r.get_platform())?,
            record_keys: text_list(r.get_record_keys())?,
        })
    }
}

/// 0x0311 UnregisterPushRelay (sealed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnregisterPushRelay {
    pub device_push_token: String,
}

impl UnregisterPushRelay {
    pub(super) fn write(&self, mut b: unregister_push_relay::Builder<'_>) {
        b.set_device_push_token(&self.device_push_token);
    }

    pub(super) fn read(r: unregister_push_relay::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            device_push_token: text(r.get_device_push_token())?,
        })
    }
}

/// 0x0312 WakeNotify (sealed): content-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WakeNotify {
    /// The relay's clock, seconds.
    pub ts: u64,
}

impl WakeNotify {
    pub(super) fn write(&self, mut b: wake_notify::Builder<'_>) {
        b.set_ts(self.ts);
    }

    pub(super) fn read(r: wake_notify::Reader<'_>) -> Self {
        Self { ts: r.get_ts() }
    }
}
