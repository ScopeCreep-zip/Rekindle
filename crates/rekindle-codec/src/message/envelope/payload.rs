//! The 1:1 message payload enum and rich-presence game info.

use serde::{Deserialize, Serialize};

/// The type of message contained in the envelope payload (after decryption).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MessagePayload {
    /// Direct 1:1 chat message.
    DirectMessage {
        body: String,
        reply_to: Option<Vec<u8>>,
    },
    /// Channel message (community text channel).
    ChannelMessage {
        channel_id: String,
        body: String,
        reply_to: Option<Vec<u8>>,
    },
    /// Typing indicator.
    TypingIndicator { typing: bool },
    /// Friend request.
    FriendRequest {
        display_name: String,
        message: String,
        prekey_bundle: Vec<u8>,
        /// Sender's private profile DHT key (for presence watching).
        profile_dht_key: String,
        /// Sender's current route blob (for immediate contact).
        route_blob: Vec<u8>,
        /// Sender's mailbox DHT key (for route discovery after reconnect).
        mailbox_dht_key: String,
        /// Correlation token linking this request back to a specific invite.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        invite_id: Option<String>,
    },
    /// Friend request acceptance.
    FriendAccept {
        prekey_bundle: Vec<u8>,
        /// Acceptor's private profile DHT key.
        profile_dht_key: String,
        /// Acceptor's current route blob.
        route_blob: Vec<u8>,
        /// Acceptor's mailbox DHT key.
        mailbox_dht_key: String,
        /// Initiator's X25519 ephemeral public key (for responder-side PQXDH).
        ephemeral_key: Vec<u8>,
        /// Which of the responder's signed prekeys was used by the initiator.
        signed_prekey_id: u32,
        /// Which of the responder's one-time prekeys was consumed (if any).
        one_time_prekey_id: Option<u32>,
        /// ML-KEM-768 ciphertext (Phase 3b PQXDH — encapsulated to the
        /// responder's PQ prekey; empty in the unlikely degenerate path).
        ml_kem_ciphertext: Vec<u8>,
        /// Which of the responder's one-time ML-KEM prekeys was consumed,
        /// or `None` if the last-resort PQ prekey was used instead.
        used_ot_pqpk_id: Option<u32>,
    },
    /// Friend request rejection.
    FriendReject,
    /// Sent to remaining friends after profile key rotation (block/unfriend).
    ProfileKeyRotated { new_profile_dht_key: String },
    /// Lightweight ACK confirming a `FriendRequest` was received and stored.
    /// Does NOT mean acceptance — just delivery confirmation.
    FriendRequestReceived,
    /// Presence update (status, game info).
    PresenceUpdate {
        status: u8,
        game_info: Option<GameInfo>,
    },
    /// Notify the peer that we have removed them as a friend.
    Unfriended,
    /// ACK confirming an `Unfriended` message was received and processed.
    UnfriendedAck,
    /// Strand Relay (architecture §13.2 step 2): a friend offers a dedicated
    /// relay route. The recipient appends the blob to their published relay
    /// pool so other contacts who can't reach them directly can route via
    /// this friend. The friend can revoke later via `RelayWithdraw`.
    RelayOffer {
        /// Opaque Veilid private-route blob created by the relay friend
        /// for forwarding-only use (kept distinct from her personal route).
        relay_route_blob: Vec<u8>,
        /// Hex-encoded Ed25519 public key of the friend volunteering to relay.
        relay_pseudonym: String,
    },
    /// Strand Relay revocation: the relay friend withdraws her offer.
    RelayWithdraw { relay_pseudonym: String },
    /// Bob's `app_call` reply to Carol confirming her `RelayOffer` was
    /// persisted into his relay pool (architecture §13.2 step 3).
    RelayOfferAck {
        ok: bool,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
    /// Strand Relay forward request (architecture §13.3 step 2): Alice→Carol.
    /// Carol sees a friend (`target_pubkey`) referenced and re-emits
    /// `inner_payload` (an entire opaque MessageEnvelope addressed to Bob)
    /// onto Bob's current route. Carol cannot read the inner content.
    RelayEnvelope {
        /// Hex-encoded Ed25519 public key of the ultimate recipient.
        target_pubkey: String,
        /// Opaque envelope bytes — the relay forwards verbatim. Encrypted
        /// to the target only, so the relay never sees plaintext.
        inner_payload: Vec<u8>,
    },
    /// 2-party DM invite (architecture §27.1): Alice → Bob. Carries the
    /// SMPL record key and slot seed; the MEK is *not* in the payload —
    /// both peers derive it deterministically via X25519 ECDH from
    /// their identity keys.
    DmInvite {
        record_key: String,
        slot_seed: Vec<u8>,
        alice_pseudonym: String,
        alice_subkey: u32,
        bob_subkey: u32,
    },
    /// Bob accepts a DM invite (architecture §27.1 line 2917).
    /// Returned as the `app_call` reply to a `DmInvite` so Alice's
    /// `start_dm` future resolves with confirmation.
    DmAccept { record_key: String },
    /// Bob declines a DM invite (architecture §27.1).
    DmDecline {
        record_key: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
    /// Group DM invite (architecture §27.2): MEK is wrapped per
    /// recipient with X25519 because ECDH only works pairwise.
    GroupDmInvite {
        record_key: String,
        slot_seed: Vec<u8>,
        initiator_pseudonym: String,
        /// JSON-encoded `Vec<rekindle_dm::GroupDmParticipant>` to keep the
        /// envelope crate dependency-free (`rekindle-dm` lives at Tier 7).
        participants_json: String,
        wrapped_mek: Vec<u8>,
        mek_generation: u32,
    },
    /// One side leaving a DM (graceful close).
    DmLeave { record_key: String },
    /// Mobile Push Relay registration (architecture §17.3 Tier 3).
    /// A mobile client asks a headless `veilid-server` push relay to
    /// watch a list of DHT record keys on its behalf and forward
    /// content-free wake signals via FCM/APNs (`{"t":"wake"}`). The
    /// relay never sees ciphertext or metadata about what changed —
    /// only that *some* registered record fired.
    RegisterPushRelay {
        /// Hex-encoded device push token (FCM registration id, APNs
        /// device token, or opaque ID for self-hosted relays).
        device_push_token: String,
        /// Platform identifier ("fcm", "apns", "self") for routing.
        platform: String,
        /// Veilid record keys (string-encoded) the relay should watch.
        record_keys: Vec<String>,
    },
    /// Mobile Push Relay revoke. Sent on logout or when the device
    /// invalidates its push token.
    UnregisterPushRelay { device_push_token: String },
    /// Wake signal — relay → mobile via FCM/APNs (out-of-band) or
    /// directly via Veilid `app_message` for desktop testing. The
    /// payload is intentionally empty of metadata: the client
    /// re-fetches the relevant records itself.
    WakeNotify {
        /// Server-side timestamp (seconds) so the client can detect
        /// stale wakes after device sleep.
        ts: u64,
    },
    /// Strand Relay presence caching (architecture §13.5): a peer asks
    /// us "do you know `target_pubkey`'s current status?". We respond
    /// from our own friend-presence state if `target_pubkey` is a
    /// friend we relay for; otherwise we drop. Faster than a DHT
    /// lookup (the social CDN pattern).
    StatusRequest { target_pubkey: String },
    /// Wave 13 — direct call invitation. Travels as fire-and-forget
    /// `app_message`, mirroring the FriendRequest handshake that already
    /// works in this codebase. Replaces the old `CallOffer` (which was
    /// shipped via `app_call` and forced the caller to block on a 30 s
    /// inline reply — wrong primitive for human reaction time, fights
    /// NAT rebind / route refresh, no reference chat app does it that
    /// way).
    ///
    /// Caller flow: insert CallState=Outgoing, fire CallInvite, return.
    /// Receiver flow: process_envelope → handle_incoming_invite →
    /// emit ChatEvent::IncomingCall + ring + surface window.
    /// User-accept: receiver fires CallAccept (separate envelope).
    /// User-decline: receiver fires CallDecline (separate envelope).
    /// 30 s with no reply: each side independently times out and
    /// fires CallTimedOut / CallMissed.
    CallInvite {
        /// Hex-encoded 16-byte random call identifier (32 chars).
        call_id: String,
        /// 0 = audio, 1 = video. Matches `rekindle_calls::CallKind::as_u8`.
        offer_kind: u8,
        /// Initiator's hex-encoded Ed25519 identity key. Used by the
        /// responder to look up display name + avatar.
        initiator_pubkey: String,
        /// Initiator's ephemeral X25519 public key (32 bytes). The
        /// receiver derives the shared call_key via X25519 ECDH +
        /// HKDF-SHA256 once they accept and learn the
        /// acceptor_x25519_pub from the receiver-side X25519 keypair.
        initiator_x25519_pub: Vec<u8>,
        /// Unix milliseconds when the ring should be considered missed.
        /// Initiator sets `now + 30_000`. Each side enforces locally.
        expires_at_ms: u64,
        /// Phase 5 — codecs the initiator's WebView can DECODE
        /// (preference-ordered wire strings: "vp9" / "vp8" / "h264").
        /// The acceptor's video sender intersects its encode set
        /// against this list. `serde(default)` covers an invite sent
        /// before the initiator's probe ran — receivers treat empty as
        /// "unknown, assume vp9 floor".
        #[serde(default)]
        video_decode_codecs: Vec<String>,
    },
    /// Wave 13 — optional alerting ack: "I got the invite, I'm ringing
    /// the user now." Lets the caller's UI distinguish "in transit"
    /// from "ringing" (Discord/Signal don't bother but it's cheap and
    /// informative). Best-effort; loss is acceptable.
    CallRinging { call_id: String },
    /// Wave 13 — receiver's accept (was an inline RPC reply in the old
    /// CallOffer/app_call design; now its own fire-and-forget envelope
    /// over `app_message`). Carries the responder's X25519 public key
    /// so the caller can derive the same call_key via ECDH + HKDF.
    /// Sent AFTER the receiver's local voice session is up so the call
    /// is bidirectional from the moment the caller acts on the accept.
    CallAccept {
        call_id: String,
        acceptor_x25519_pub: Vec<u8>,
        /// Phase 5 — codecs the acceptor's WebView can DECODE (mirror
        /// of `CallInvite::video_decode_codecs`, same wire strings and
        /// empty-means-unknown semantics).
        #[serde(default)]
        video_decode_codecs: Vec<String>,
    },
    /// Wave 13 — receiver's explicit decline (was an inline RPC reply;
    /// now standalone `app_message`).
    CallDecline {
        call_id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
    /// Hangup — sent by either party to end an Active call. Receiver
    /// removes the call from `state.active_calls` and emits
    /// `ChatEvent::CallEnded` so the frontend can clear `activeCall`.
    /// Wave 13 W13.9 — also handles Dialing / Incoming / Connecting
    /// state so a cancel-while-ringing race cleans up cleanly on both
    /// sides.
    CallEnd {
        call_id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
    /// Wave 12 W12.6 — mid-call media state change. Sent by either party
    /// when the local mute / camera / screen-share toggle flips so the
    /// peer's UI can reactively show or hide the corresponding tile.
    /// Does NOT renegotiate the `call_key` — this is a non-renegotiation
    /// state ping, not a fresh handshake. Travels via `app_message`
    /// after the call is established (post-CallAccept).
    CallMediaState {
        call_id: String,
        /// Sender's microphone is currently producing audio (i.e. they
        /// are NOT muted). Receivers can use this for a UI hint;
        /// authoritative mute is enforced by the sender ceasing to
        /// transmit audio frames.
        audio: bool,
        /// Sender's camera is on and they're transmitting VP9 frames
        /// for `track_label = "camera"`. Receivers mount a video tile
        /// when this flips true.
        video: bool,
        /// Sender's screen-share is on and they're transmitting VP9
        /// frames for `track_label = "screen"`. Independent of `video`
        /// so the two streams can co-exist.
        screen: bool,
        /// Wall-clock millis at the sender; used by receivers to drop
        /// out-of-order updates (last-write-wins on this single field).
        timestamp_ms: u64,
    },
    /// Wave 12 W12.11 — in-call emoji reaction. Receivers float the
    /// emoji over their call panel for ~2 s. Fire-and-forget via
    /// `app_message`; loss is acceptable (it's eye-candy, not state).
    CallReaction {
        call_id: String,
        /// Single grapheme cluster (e.g. "👍", "❤️"). Receivers cap
        /// length to a small bound to defeat oversized-emoji DoS via
        /// hand-crafted clients.
        emoji: String,
        /// Sender's millis-since-epoch — receivers use this to dedup
        /// rapid-fire spamming and to drop reactions that arrived
        /// after their TTL window.
        timestamp_ms: u64,
    },
    /// Wave 12 W12.9 — group call offer. The initiator sends one
    /// envelope PER invitee, each with that invitee's per-recipient
    /// `wrapped_call_key` (X25519 + HKDF + AES-256-GCM). Other
    /// invitees can't decrypt this recipient's wrap, so the call_key
    /// stays scoped to the explicit invite list. Travels via
    /// `app_call` so the responder's accept/decline returns inline.
    GroupCallOffer {
        call_id: String,
        /// 0 = audio, 1 = video.
        offer_kind: u8,
        /// Initiator's hex-encoded Ed25519 identity key.
        initiator_pubkey: String,
        /// Initiator's ephemeral X25519 public key (32 bytes). Used by
        /// the recipient to derive the same wrap_key the initiator
        /// used to seal `wrapped_call_key`.
        initiator_x25519_pub: Vec<u8>,
        /// Hex pubkeys of every invitee, included so each recipient
        /// can render the participant grid before they accept and so
        /// late joins know who's expected.
        participants: Vec<String>,
        /// 60-byte (12 nonce + 32 ciphertext + 16 tag) per-recipient
        /// sealed call_key. Only THIS recipient can decrypt — see
        /// rekindle_calls::group::wrap_call_key.
        wrapped_call_key: Vec<u8>,
        /// Unix millis when the ring expires.
        expires_at_ms: u64,
    },
    /// Wave 12 W12.9 — reply to GroupCallOffer carrying the
    /// acceptor's identity so other participants can be told who
    /// joined. Returned as the inline `app_call` reply.
    GroupCallAccept {
        call_id: String,
        acceptor_pubkey: String,
    },
    /// Wave 12 W12.9 — reply rejecting a GroupCallOffer.
    GroupCallDecline {
        call_id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
    /// Wave 12 W12.9 — gossiped notice that another participant has
    /// joined a group call already in progress (post-acceptance). Not
    /// authoritative; receivers verify the participant is in the
    /// call's invite list before adding to their grid.
    GroupCallParticipantJoined {
        call_id: String,
        participant_pubkey: String,
    },
    /// Wave 12 W12.9 — gossiped notice that a participant has left.
    /// Receivers prune their grid; voice topology re-elects mesh / SFU
    /// as needed.
    GroupCallParticipantLeft {
        call_id: String,
        participant_pubkey: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
    /// P3.3 session renewal — Alice → Bob: "my Signal session for you is
    /// broken (decrypt failures, lost on upgrade, deliberate reset);
    /// please re-handshake. Here's my fresh PreKeyBundle." This is the
    /// REQUEST half of the user-confirmed renewal flow.
    ///
    /// Receiver emits NotificationEvent::SessionResetRequested for user
    /// review (per vulnerable-user safety stance — no auto-process). The
    /// user confirms after verifying the sender's safety number
    /// out-of-band, then `accept_session_reset` consumes the stored
    /// bundle, calls establish_session(sender, bundle), and sends back
    /// SessionResetAccept with the X3DH metadata.
    SessionResetRequest {
        /// Requester's serialized PreKeyBundle (JSON of
        /// rekindle_crypto::signal::PreKeyBundle). Receiver feeds this
        /// into establish_session(sender, bundle) on user-accept.
        our_prekey_bundle: Vec<u8>,
    },
    /// P3.3 session renewal — Bob → Alice: "I accepted, here's the X3DH
    /// metadata so you can call respond_to_session and complete the
    /// renewal." Alice verifies our_identity_key against her stored
    /// trusted-identity record (TOFU update on user confirmation) before
    /// applying the new session.
    SessionResetAccept {
        /// Bob's X25519 ephemeral public key generated during his
        /// establish_session. Alice uses this in respond_to_session.
        ephemeral_key: Vec<u8>,
        /// Which of Alice's signed prekeys Bob used.
        signed_prekey_id: u32,
        /// Which of Alice's one-time prekeys Bob consumed (if any).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        one_time_prekey_id: Option<u32>,
        /// Bob's identity key (X25519 form, 32 bytes). Alice verifies
        /// this matches her stored trusted-identity for Bob.
        our_identity_key: Vec<u8>,
        /// ML-KEM-768 ciphertext (Phase 3b PQXDH — encapsulated to one of
        /// Alice's PQ prekeys during Bob's `establish_session`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ml_kem_ciphertext: Vec<u8>,
        /// Which of Alice's one-time ML-KEM prekeys Bob consumed, or
        /// `None` if the last-resort PQ prekey was used.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        used_ot_pqpk_id: Option<u32>,
    },
    /// P3.3 session renewal — Bob → Alice: "I declined the reset
    /// request." Alice's UI surfaces the decline; her existing local
    /// session state stays whatever it was (broken from her side, but
    /// Bob continues to use his existing session).
    SessionResetDecline {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
    /// Reply to a `StatusRequest`. Empty `status` means "I don't have
    /// data for this peer" so the requester can short-circuit.
    StatusResponse {
        target_pubkey: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status_message: Option<String>,
        /// Unix timestamp (seconds) of the last presence update we saw
        /// for this peer. Lets the requester reject stale snapshots.
        last_seen: u64,
        /// The peer's most recent route blob, so the requester can
        /// short-circuit DHT route lookup as well.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        route_blob: Vec<u8>,
    },
    /// W11.4 (P6.2) — DM video frame fragment for 1:1 video calls.
    ///
    /// The frame's `ciphertext` is encrypted by the existing
    /// `send_envelope_to_peer` Signal Double Ratchet path before
    /// transport — this variant is the inner plaintext payload that
    /// gets wrapped. Receivers reassemble fragments by `(stream_id,
    /// frame_seq)` until `fragment_count` chunks accumulate, then hand
    /// off to the WebCodecs VideoDecoder.
    ///
    /// We use the same VP9 + 480p shape as community video but route
    /// 1:1 instead of mesh. Mirrors `CommunityEnvelope::VideoFragment`
    /// (architecture §10.6) without MEK or community context: the DM
    /// session keys (Signal) cover both authentication and
    /// confidentiality.
    DmVideoFragment {
        /// 16-byte stream identifier — stable for the lifetime of one
        /// camera or screen-share session within a call.
        stream_id: [u8; 16],
        /// Monotonic frame counter per stream, starting at 0.
        frame_seq: u32,
        /// 0-based fragment index within this frame.
        fragment_index: u16,
        /// Total fragments for this frame. Receivers wait until they
        /// have all `fragment_count` to reassemble.
        fragment_count: u16,
        /// True for keyframes (decoder bootstrapping).
        keyframe: bool,
        /// Codec wire string ("vp9" | "vp8" | "h264") — the receiver
        /// configures its decoder from this tag (RTP payload-type
        /// analog). Authenticity comes from the Signal session layer.
        codec: String,
        /// Encoder-provided presentation timestamp.
        timestamp: u32,
        /// VP9 chunk bytes (no nested encryption — Signal layer
        /// encrypts the whole envelope before transport).
        chunk: Vec<u8>,
    },
}

/// Game information for rich presence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameInfo {
    pub game_id: u32,
    pub game_name: String,
    pub server_info: Option<String>,
    pub elapsed_seconds: u32,
    /// Direct server address ("ip:port") for join-game functionality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_address: Option<String>,
}

#[cfg(test)]
mod call_payload_tests {
    use super::*;

    /// Phase 5 — `video_decode_codecs` rides CallInvite/CallAccept and
    /// round-trips through the JSON wire form.
    #[test]
    fn call_invite_and_accept_decode_codecs_roundtrip() {
        let invite = MessagePayload::CallInvite {
            call_id: "c1".into(),
            offer_kind: 1,
            initiator_pubkey: "ab".repeat(32),
            initiator_x25519_pub: vec![7u8; 32],
            expires_at_ms: 123,
            video_decode_codecs: vec!["vp9".into(), "h264".into()],
        };
        let json = serde_json::to_string(&invite).unwrap();
        let back: MessagePayload = serde_json::from_str(&json).unwrap();
        let MessagePayload::CallInvite {
            video_decode_codecs,
            ..
        } = back
        else {
            panic!("wrong variant");
        };
        assert_eq!(video_decode_codecs, vec!["vp9", "h264"]);

        let accept = MessagePayload::CallAccept {
            call_id: "c1".into(),
            acceptor_x25519_pub: vec![9u8; 32],
            video_decode_codecs: vec!["vp8".into()],
        };
        let json = serde_json::to_string(&accept).unwrap();
        let back: MessagePayload = serde_json::from_str(&json).unwrap();
        let MessagePayload::CallAccept {
            video_decode_codecs,
            ..
        } = back
        else {
            panic!("wrong variant");
        };
        assert_eq!(video_decode_codecs, vec!["vp8"]);
    }

    /// `serde(default)` — an invite serialized before the field existed
    /// (or from a sender whose probe hadn't run) deserializes to an
    /// empty list, which senders treat as the VP9 floor.
    #[test]
    fn call_payloads_missing_decode_codecs_default_empty() {
        let invite_json = format!(
            r#"{{"type":"CallInvite","call_id":"c1","offer_kind":0,"initiator_pubkey":"{}","initiator_x25519_pub":[1,2],"expires_at_ms":5}}"#,
            "ab".repeat(32)
        );
        let back: MessagePayload = serde_json::from_str(&invite_json).unwrap();
        let MessagePayload::CallInvite {
            video_decode_codecs,
            ..
        } = back
        else {
            panic!("wrong variant");
        };
        assert!(video_decode_codecs.is_empty());

        let accept_json = r#"{"type":"CallAccept","call_id":"c1","acceptor_x25519_pub":[1,2]}"#;
        let back: MessagePayload = serde_json::from_str(accept_json).unwrap();
        let MessagePayload::CallAccept {
            video_decode_codecs,
            ..
        } = back
        else {
            panic!("wrong variant");
        };
        assert!(video_decode_codecs.is_empty());
    }
}
