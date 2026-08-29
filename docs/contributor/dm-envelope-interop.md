# DM envelope interop audit — desktop vs daemon track

Companion to the ratchet convergence (`rekindle_crypto::signal::ratchet`,
which made the two tracks' Signal sessions wire-compatible and proved it
with `cross_track_session_interop`). This documents the layer ABOVE the
ratchet: the signed envelope each track wraps around a DM before
`app_message`. **Result: not interoperable, at three separate layers.**

## The two wire formats

| Layer | Desktop track | Daemon track |
|---|---|---|
| Framing | raw JSON bytes (`serde_json::to_vec(&envelope)`, `src-tauri/src/services/message_service/transport.rs:139`) | type-tagged frame: `frame::encode(TypeId, postcard(SignedPayload))` (`crates/rekindle-transport/src/broadcast/send.rs` `send_dm`) |
| Envelope struct | `MessageEnvelope { sender_key, timestamp, nonce, payload, signature }` (`crates/rekindle-protocol/src/messaging/envelope.rs`) | `SignedPayload { sender_key_hex, timestamp, seq, correlation_id, payload, signature }` (`crates/rekindle-transport/src/crypto/envelope.rs`) |
| Signature domain | Ed25519 over serde_json of a signable struct (`timestamp \|\| nonce \|\| payload`) | Ed25519 over `timestamp(8 LE) \|\| seq(8 LE) \|\| correlation_id_len(4 LE) \|\| correlation_id \|\| payload` |
| Inner payload | `MessagePayload` enum (JSON) | `DmPayload` enum (`crates/rekindle-transport/src/payload/dm.rs`, postcard) |

A desktop client receiving a daemon frame fails at `serde_json::from_slice`;
a daemon client receiving desktop JSON fails at `frame::decode`. Nothing
reaches the (now-shared) ratchet layer.

## Assessment

- The daemon format is the better target: length-framed + type-tagged
  (dispatch without trial deserialization), postcard (compact,
  deterministic), envelope-level `seq`/`correlation_id` feeding the
  receiver-side dedup primitive (`SeqTracker`) — the desktop's random
  `nonce` cannot express ordered dedup.
- The desktop format's only unique property is human-readable JSON,
  which is not a wire requirement.

## Convergence plan (step 2 — its own effort, not this pass)

1. Home the envelope in `rekindle-codec` (Tier 3 — the serialization
   crate that exists for exactly this): move `SignedPayload` + its
   sign/verify + the frame layer there; re-export from
   `rekindle-transport` for existing daemon call sites.
2. Migrate the desktop send path (`message_service/transport.rs`,
   `relay/send.rs`, `sync_adapter/attempt.rs`) and receive path
   (`veilid_service` app_message handler) onto it. The desktop's
   `MessagePayload` variants map onto `DmPayload` (they cover the same
   operations; reconcile field-by-field during the migration).
3. Pre-release (architecture rule B14): no dual-read shim — cut over in
   one release, as with the ratchet convergence.
4. Add `cross_track_dm_envelope_interop` mirroring the ratchet interop
   test: build on one track's sender, parse + verify + decrypt on the
   other track's receiver.

Until step 2 lands, desktop↔daemon DMs remain non-functional at the
envelope layer even though the session/ratchet layer beneath is now
compatible.

## Non-goals

`SignedEnvelope` (`crates/rekindle-protocol/src/dht/community/envelope.rs`)
signs SMPL DHT records, not transport messages — different job, stays
separate (it is a 600-LOC-split candidate, nothing more).
