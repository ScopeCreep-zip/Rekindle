use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// Wire format envelope wrapping all messages sent over Veilid.
///
/// The payload is E2E encrypted (Signal Protocol for DMs, MEK for channels).
/// This envelope provides sender identification and integrity verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEnvelope {
    /// Sender's Ed25519 public key (32 bytes).
    pub sender_key: Vec<u8>,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// Unique message nonce (for deduplication and ordering).
    pub nonce: Vec<u8>,
    /// Encrypted payload (ciphertext).
    pub payload: Vec<u8>,
    /// Ed25519 signature over (timestamp || nonce || payload).
    pub signature: Vec<u8>,
}

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
        /// Initiator's X25519 ephemeral public key (for responder-side X3DH).
        ephemeral_key: Vec<u8>,
        /// Which of the responder's signed prekeys was used by the initiator.
        signed_prekey_id: u32,
        /// Which of the responder's one-time prekeys was consumed (if any).
        one_time_prekey_id: Option<u32>,
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
    RelayWithdraw {
        relay_pseudonym: String,
    },
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
    DmAccept {
        record_key: String,
    },
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
    DmLeave {
        record_key: String,
    },
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
    UnregisterPushRelay {
        device_push_token: String,
    },
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
    StatusRequest {
        target_pubkey: String,
    },
    /// Direct call offer (architecture §10.10 / Plan §Failure 5).
    /// The initiator generates an ephemeral X25519 keypair, sends the
    /// public key here, and awaits `CallAccept` (with the responder's
    /// public key) so both sides can derive the same `call_key` via
    /// HKDF-SHA256 over the ECDH shared secret. Shipped via
    /// `app_call` so the responder's accept/decline returns inline.
    CallOffer {
        /// Hex-encoded 16-byte random call identifier (32 chars).
        call_id: String,
        /// 0 = audio, 1 = video. Matches `rekindle_calls::CallKind::as_u8`.
        offer_kind: u8,
        /// Initiator's hex-encoded Ed25519 identity key. Used by the
        /// responder to look up display name + avatar.
        initiator_pubkey: String,
        /// Initiator's ephemeral X25519 public key (32 bytes).
        initiator_x25519_pub: Vec<u8>,
        /// Unix milliseconds when the ring should be considered missed.
        /// Initiator sets `now + 30_000`.
        expires_at_ms: u64,
    },
    /// Reply to `CallOffer` carrying the responder's X25519 public key
    /// so the initiator can finish key derivation. Returned as the
    /// `app_call` reply, never sent unsolicited.
    CallAccept {
        call_id: String,
        acceptor_x25519_pub: Vec<u8>,
    },
    /// Reply to `CallOffer` rejecting the call. Same shape as
    /// `DmDecline` so the dispatcher can mirror existing patterns.
    CallDecline {
        call_id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
    /// C2 hangup — sent by either party to end an Active call. Receiver
    /// removes the call from `state.active_calls` and emits
    /// `ChatEvent::CallEnded` so the frontend can clear `activeCall`.
    /// Distinct from `CallDecline` (which is the inline `app_call` reply
    /// to a CallOffer) — this one travels via `app_message` after the
    /// call is established.
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
        /// True for VP9 keyframes (decoder bootstrapping).
        keyframe: bool,
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

// ---------------------------------------------------------------------------
// Invite blob types
// ---------------------------------------------------------------------------

/// A signed invite blob that contains everything needed for initial contact.
///
/// Encoded as JSON, signed with Ed25519, then base64url-encoded for sharing
/// as a `rekindle://` URL or plain string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteBlob {
    /// Sender's Ed25519 public key (hex).
    pub public_key: String,
    /// Sender's display name.
    pub display_name: String,
    /// Sender's mailbox DHT record key (for reading route blob).
    pub mailbox_dht_key: String,
    /// Sender's private profile DHT record key (for presence watching).
    pub profile_dht_key: String,
    /// Sender's current route blob (for immediate contact, may be stale).
    pub route_blob: Vec<u8>,
    /// Sender's Signal `PreKeyBundle` (serialized JSON).
    pub prekey_bundle: Vec<u8>,
    /// Correlation token linking this invite to tracked outgoing invites.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite_id: Option<String>,
    /// Unix epoch milliseconds when the invite was minted (B11 hardening).
    /// Recipients enforce a max-age policy via [`check_invite_recency`] so
    /// leaked / harvested links can't be redeemed indefinitely. Older
    /// invites (pre-issued_at-field) decode with `0` here and are rejected
    /// as stale by [`check_invite_recency`] — no legacy fallback.
    #[serde(default)]
    pub issued_at: u64,
    /// Ed25519 signature over the JSON of all fields above.
    pub signature: Vec<u8>,
}

/// Create a signed invite blob from identity credentials.
///
/// Signs over a JSON-serialized form of the invite data (excluding the
/// signature field itself) using the Ed25519 secret key. `issued_at_ms` is
/// covered by the signature so a third party cannot back-date a leaked
/// blob to extend its useful life past the recipient's recency window.
pub fn create_invite_blob(
    secret_key: &[u8; 32],
    public_key: &str,
    display_name: &str,
    mailbox_dht_key: &str,
    profile_dht_key: &str,
    route_blob: &[u8],
    prekey_bundle: &[u8],
    invite_id: Option<&str>,
    issued_at_ms: u64,
) -> InviteBlob {
    use ed25519_dalek::{Signer, SigningKey};

    let signing_key = SigningKey::from_bytes(secret_key);

    // Build the signable payload (all fields except signature)
    let signable = serde_json::json!({
        "public_key": public_key,
        "display_name": display_name,
        "mailbox_dht_key": mailbox_dht_key,
        "profile_dht_key": profile_dht_key,
        "route_blob": route_blob,
        "prekey_bundle": prekey_bundle,
        "invite_id": invite_id,
        "issued_at": issued_at_ms,
    });
    let signable_bytes = serde_json::to_vec(&signable).unwrap_or_default();
    let signature = signing_key.sign(&signable_bytes);

    InviteBlob {
        public_key: public_key.to_string(),
        display_name: display_name.to_string(),
        mailbox_dht_key: mailbox_dht_key.to_string(),
        profile_dht_key: profile_dht_key.to_string(),
        route_blob: route_blob.to_vec(),
        prekey_bundle: prekey_bundle.to_vec(),
        invite_id: invite_id.map(str::to_string),
        issued_at: issued_at_ms,
        signature: signature.to_bytes().to_vec(),
    }
}

/// Verify the Ed25519 signature on an invite blob.
///
/// Returns `Ok(())` if the signature is valid, `Err` otherwise.
pub fn verify_invite_blob(blob: &InviteBlob) -> Result<(), String> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let pub_bytes =
        hex::decode(&blob.public_key).map_err(|e| format!("invalid public key hex: {e}"))?;
    let pub_array: [u8; 32] = pub_bytes
        .try_into()
        .map_err(|_| "public key must be 32 bytes".to_string())?;
    let verifying_key =
        VerifyingKey::from_bytes(&pub_array).map_err(|e| format!("invalid public key: {e}"))?;

    let sig_array: [u8; 64] = blob
        .signature
        .clone()
        .try_into()
        .map_err(|_| "signature must be 64 bytes".to_string())?;
    let signature = Signature::from_bytes(&sig_array);

    // Reconstruct the signable payload
    let signable = serde_json::json!({
        "public_key": blob.public_key,
        "display_name": blob.display_name,
        "mailbox_dht_key": blob.mailbox_dht_key,
        "profile_dht_key": blob.profile_dht_key,
        "route_blob": blob.route_blob,
        "prekey_bundle": blob.prekey_bundle,
        "invite_id": blob.invite_id,
        "issued_at": blob.issued_at,
    });
    let signable_bytes = serde_json::to_vec(&signable).unwrap_or_default();

    verifying_key
        .verify(&signable_bytes, &signature)
        .map_err(|e| {
            // Most signature failures users hit in practice are version
            // mismatches: the sender's build pre-dates the B11 issued_at
            // field, so the canonical bytes the receiver reconstructs
            // (which always include `"issued_at": 0`) don't match what
            // the sender signed. Surface this hint instead of just the
            // raw cryptographic error so the user knows what to do.
            if blob.issued_at == 0 {
                format!(
                    "invite link is from an older app version (no issued_at). \
                     Ask the sender to regenerate the invite on a current build. \
                     (raw: {e})"
                )
            } else {
                format!("invalid invite signature: {e}")
            }
        })
}

#[cfg(test)]
mod invite_blob_tests {
    use super::*;

    fn make_secret() -> [u8; 32] {
        // Fixed test secret so the test is deterministic.
        let mut s = [0u8; 32];
        for (i, b) in s.iter_mut().enumerate() {
            *b = (i as u8).wrapping_add(1);
        }
        s
    }

    fn pubkey_hex(secret: &[u8; 32]) -> String {
        use ed25519_dalek::SigningKey;
        let sk = SigningKey::from_bytes(secret);
        hex::encode(sk.verifying_key().to_bytes())
    }

    #[test]
    fn create_then_verify_round_trips_with_issued_at() {
        let secret = make_secret();
        let pk = pubkey_hex(&secret);
        let blob = create_invite_blob(
            &secret,
            &pk,
            "Alice",
            "VLD0:mailbox-key",
            "VLD0:profile-key",
            &[1u8, 2, 3],
            &[10u8, 20, 30, 40],
            Some("inv-123"),
            1_715_000_000_000,
        );
        verify_invite_blob(&blob).expect("freshly-minted blob must verify on the same build");
        assert_eq!(blob.issued_at, 1_715_000_000_000);
    }

    #[test]
    fn verify_rejects_pre_b11_blob_missing_issued_at() {
        // Simulate a pre-B11 sender by signing without issued_at, then
        // verifying with the new code path. The signature reconstruction
        // includes issued_at:0 (serde default for missing field), so the
        // canonical bytes differ from what the sender signed → mismatch.
        // This is the no-legacy-compat behavior: the user must regenerate
        // the invite on a current build.
        use ed25519_dalek::{Signer, SigningKey};
        let secret = make_secret();
        let signing = SigningKey::from_bytes(&secret);
        let pk = pubkey_hex(&secret);
        let pre_b11_signable = serde_json::json!({
            "public_key": pk,
            "display_name": "Alice",
            "mailbox_dht_key": "VLD0:mailbox",
            "profile_dht_key": "VLD0:profile",
            "route_blob": &[1u8, 2, 3],
            "prekey_bundle": &[10u8, 20],
            "invite_id": Some("inv-old"),
        });
        let signable_bytes = serde_json::to_vec(&pre_b11_signable).unwrap();
        let signature = signing.sign(&signable_bytes);
        let blob = InviteBlob {
            public_key: pk,
            display_name: "Alice".to_string(),
            mailbox_dht_key: "VLD0:mailbox".to_string(),
            profile_dht_key: "VLD0:profile".to_string(),
            route_blob: vec![1u8, 2, 3],
            prekey_bundle: vec![10u8, 20],
            invite_id: Some("inv-old".to_string()),
            issued_at: 0, // pre-B11 sender didn't include this field
            signature: signature.to_bytes().to_vec(),
        };
        let err = verify_invite_blob(&blob).expect_err("pre-B11 blob must be rejected");
        // The error message hints at the version mismatch instead of
        // showing the raw cryptographic error.
        assert!(
            err.contains("older app version"),
            "got: {err}",
        );
    }

    #[test]
    fn check_recency_within_window() {
        let secret = make_secret();
        let pk = pubkey_hex(&secret);
        let blob = create_invite_blob(
            &secret,
            &pk,
            "Alice",
            "mb",
            "pr",
            &[],
            &[],
            None,
            1_715_000_000_000,
        );
        // 1 day after issuance
        let now = blob.issued_at + 24 * 3600 * 1000;
        check_invite_recency(&blob, now, 7 * 24 * 3600).expect("within window");
    }

    #[test]
    fn check_recency_rejects_zero_issued_at() {
        let secret = make_secret();
        let pk = pubkey_hex(&secret);
        let mut blob = create_invite_blob(
            &secret,
            &pk,
            "Alice",
            "mb",
            "pr",
            &[],
            &[],
            None,
            1_715_000_000_000,
        );
        blob.issued_at = 0;
        let err = check_invite_recency(&blob, 1_715_000_000_000, 7 * 24 * 3600)
            .expect_err("issued_at=0 must be rejected");
        assert!(
            err.contains("missing issued_at"),
            "got: {err}",
        );
    }
}

/// Reject an invite that was minted more than `max_age_secs` ago.
///
/// Defense-in-depth alongside the sender-side `mark_invite_responded`
/// single-use enforcement: even if an attacker harvests a link and
/// presents it to multiple receivers, the recency window caps how long
/// the harvest remains useful. Vulnerable-user safety stance: leaked
/// links shouldn't grant indefinite reach. Default policy in
/// `add_friend_from_invite` is 7 days.
///
/// `now_ms` is supplied by the caller (rather than read from the
/// system clock) so this stays as a pure protocol helper. An invite
/// with `issued_at == 0` is rejected as a pre-recency-field blob —
/// the sender must regenerate. No legacy fallback.
pub fn check_invite_recency(
    blob: &InviteBlob,
    now_ms: u64,
    max_age_secs: u64,
) -> Result<(), String> {
    if blob.issued_at == 0 {
        return Err(
            "invite is missing issued_at — please ask the sender to regenerate the invite link"
                .to_string(),
        );
    }
    let age_ms = now_ms.saturating_sub(blob.issued_at);
    let max_age_ms = max_age_secs.saturating_mul(1_000);
    if age_ms > max_age_ms {
        let age_days = age_ms / (24 * 3600 * 1000);
        let max_age_days = max_age_secs / (24 * 3600);
        return Err(format!(
            "invite expired ({age_days}d old, max {max_age_days}d) — please ask the sender to generate a new invite"
        ));
    }
    Ok(())
}

/// Encode an invite blob as a `rekindle://` URL.
pub fn encode_invite_url(blob: &InviteBlob) -> String {
    let json = serde_json::to_vec(blob).unwrap_or_default();
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&json);
    format!("rekindle://{encoded}")
}

/// Decode an invite blob from a `rekindle://` URL or raw base64 string.
pub fn decode_invite_url(url: &str) -> Result<InviteBlob, String> {
    let data = url.strip_prefix("rekindle://").unwrap_or(url);
    let json_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(data)
        .map_err(|e| format!("invalid base64: {e}"))?;
    let blob: InviteBlob =
        serde_json::from_slice(&json_bytes).map_err(|e| format!("invalid invite JSON: {e}"))?;
    Ok(blob)
}

// ---------------------------------------------------------------------------
// Community server RPC DTOs (used for Tauri IPC serialization)
// ---------------------------------------------------------------------------

// NOTE: CommunityRequest, CommunityResponse, and CommunityBroadcast enums
// have been removed. All community protocol now goes through the v2
// coordinator/ControlPayload model (see dht::community::envelope).

/// A role definition as returned by the server over RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDto {
    pub id: u32,
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    pub position: i32,
    pub hoist: bool,
    pub mentionable: bool,
    #[serde(default)]
    pub self_assignable: bool,
}

/// A community member as returned by the server in the join/rejoin response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberInfoDto {
    pub pseudonym_key: String,
    pub display_name: String,
    pub role_ids: Vec<u32>,
}

/// A banned member as returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BannedMemberDto {
    pub pseudonym_key: String,
    pub display_name: String,
    pub banned_at: u64,
}

/// A channel message as returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMessageDto {
    pub message_id: String,
    pub sender_pseudonym: String,
    pub ciphertext: Vec<u8>,
    pub mek_generation: u64,
    pub timestamp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<ReactionGroupDto>,
}

/// Helper for `skip_serializing_if` on `u32` fields that default to 0.
///
/// `serde`'s `skip_serializing_if` always passes by reference, so `&u32` is required.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// Channel info as returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfoDto {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub topic: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub slowmode_seconds: u32,
}

/// A channel category as returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryDto {
    pub id: String,
    pub name: String,
    pub sort_order: i32,
}

/// A community invite code as returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteDto {
    pub code: String,
    pub created_by: String,
    pub max_uses: Option<u32>,
    pub uses: u32,
    pub expires_at: Option<u64>,
    pub created_at: u64,
}

/// A pinned message as returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinnedMessageDto {
    pub message_id: String,
    pub channel_id: String,
    pub pinned_by: String,
    pub pinned_at: u64,
}

/// Aggregated reaction data for a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionGroupDto {
    pub emoji: String,
    pub count: u32,
    pub reactors: Vec<String>,
}

/// An audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogEntryDto {
    pub action: String,
    pub actor_pseudonym: String,
    pub target: Option<String>,
    pub details: Option<String>,
    pub timestamp: u64,
}

/// A community event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDto {
    pub id: String,
    pub title: String,
    pub description: String,
    pub creator_pseudonym: String,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub channel_id: Option<String>,
    pub max_attendees: Option<u32>,
    pub created_at: u64,
    /// Lifecycle status: "scheduled", "active", "completed", "canceled".
    pub status: String,
    pub rsvps: Vec<EventRsvpDto>,
}

/// An RSVP entry for an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRsvpDto {
    pub pseudonym_key: String,
    pub status: String,
}

/// A game server favorite in a community.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameServerDto {
    pub id: String,
    pub game_id: String,
    pub label: String,
    pub address: String,
    pub added_by: String,
    pub created_at: u64,
}

/// Unread count for a single channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadCountDto {
    pub channel_id: String,
    pub unread_count: u32,
}

/// A thread (branching conversation from a channel message).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadInfoDto {
    pub id: String,
    pub channel_id: String,
    pub name: String,
    pub starter_message_id: String,
    pub creator_pseudonym: String,
    pub created_at: u64,
    pub archived: bool,
    pub auto_archive_seconds: u32,
    pub last_message_at: u64,
    pub message_count: u32,
}
