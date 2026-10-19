# Voice Architecture

> **Status (2026-10-15).** [ADR 0015](../decisions/0015-media-engine.md) replaces the jitter buffer,
> mixer and hand-derived control loops described below with one Rust media engine built from ported
> production components (NetEQ-class playout, GoogCC trendline); the full rewrite of this page rides
> plan step E4.3.8. [ADR 0016](../decisions/0016-sender-owned-call-media-keys.md) records the media-key
> model (§"Channel media keys"). The former `src-tauri/src/services/voice/` files now live in
> `crates/rekindle-voice/src/`; the desktop glue is `src-tauri/src/services/voice_adapter/` and
> `voice_runtime.rs`.

The voice pipeline carries low-latency audio between peers in 1:1 calls,
DM/group-DM calls, and community voice channels. It is built on Veilid's
`app_message` primitive over a 3-hop Tor-class `SafetySelection::Safe`
route (anonymous sender, `LowLatency` stability), and on `cpal` for
cross-platform audio I/O.

The implementation lives in two places:

- **`crates/rekindle-voice/`** — pure-logic audio pipeline (capture,
  encode, transport, jitter, mix, playback).
- **`src-tauri/src/services/voice_adapter/`, `voice_runtime.rs`** — the
  Tauri-side glue (Veilid egress, media keys, session setup, event
  mapping). The send/receive/MCU loops, signaling, election, device
  monitor, session state and shutdown live in `rekindle-voice`.

This split lets the audio pipeline be unit-tested without Tauri or Veilid
mocks.

## End-to-end pipeline

```
                    ┌─────────────────────────┐
                    │      Microphone         │
                    └────────────┬────────────┘
                                 │ cpal::Stream (dedicated thread)
                                 ▼
                          mpsc::channel<Vec<f32>>
                                 │
                                 ▼
                    ┌─────────────────────────┐
                    │  AudioProcessor         │  RNNoise denoise + AEC3 + VAD
                    └────────────┬────────────┘
                                 │ 20 ms frames @ 48 kHz mono
                                 ▼
                    ┌─────────────────────────┐
                    │  OpusCodec (encode)     │  VoIP mode, 32 kbps, in-band FEC
                    └────────────┬────────────┘
                                 │ Vec<u8>
                                 ▼
                    ┌─────────────────────────┐
                    │  VoiceTransport (send)  │  Veilid app_message, 3-hop Safe route
                    └────────────┬────────────┘
                                 │
                              ===NETWORK===
                                 │
                    ┌────────────▼────────────┐
                    │ VeilidUpdate::AppMessage│  classified by prefix
                    └────────────┬────────────┘
                                 │ VoicePacket
                                 ▼
                    ┌─────────────────────────┐
                    │  JitterBuffer           │  adaptive 40/80/120 ms
                    └────────────┬────────────┘
                                 │
                                 ▼
                    ┌─────────────────────────┐
                    │  OpusCodec (decode)     │
                    └────────────┬────────────┘
                                 │
                                 ▼
                    ┌─────────────────────────┐
                    │  AudioMixer             │  per-participant gain, sum
                    └────────────┬────────────┘
                                 │ mpsc::channel<Vec<f32>>
                                 ▼
                          cpal::Stream (dedicated thread)
                                 │
                                 ▼
                    ┌─────────────────────────┐
                    │      Speaker            │
                    └─────────────────────────┘
```

## Threading model

`cpal::Stream` is `!Send` on macOS — audio streams must live on dedicated
OS threads. The crate bridges these threads to the Tokio runtime via
`mpsc` channels.

| Thread | Owner | Direction |
|--------|-------|-----------|
| Capture thread | `AudioCapture` | Microphone PCM → `mpsc::Sender<Vec<f32>>` |
| Playback thread | `AudioPlayback` | `mpsc::Receiver<Vec<f32>>` → speaker |
| Tokio task: send loop | `rekindle-voice/src/send_loop/` | Drains capture, runs `AudioProcessor`, encodes, transports |
| Tokio task: receive loop | `rekindle-voice/src/receive_loop/` | Reads `VoicePacket`s from dispatch, jitters, decodes |
| Tokio task: MCU loop | `rekindle-voice/src/mcu_loop.rs` | Mutual-aid SFU mixer for >4-participant calls |
| Tokio task: device monitor | `rekindle-voice/src/session/device_monitor.rs` | Watches `device_error_rx` for hot-plug / device-failed events |

All cross-thread communication uses Tokio `mpsc` channels with bounded
capacity (~100 frames ≈ 2 s of audio at 20 ms per frame). Backpressure
drops the oldest frames when consumers fall behind, preserving real-time
delivery semantics over completeness.

## Audio parameters

| Setting | Value | Rationale |
|---------|-------|-----------|
| Sample rate | 48 000 Hz | Opus's native operating rate; avoids resampling. |
| Channels | 1 (mono) | Reduces bandwidth; voice is the dominant signal. |
| Frame size | 960 samples (20 ms) | Industry-standard VoIP frame; balances latency and overhead. |
| Codec | Opus | Royalty-free, IETF-standard, low-delay speech codec. |
| Application mode | `Voip` | Tunes Opus for speech (vs. `Audio` or `LowDelay`). |
| Bitrate | 32 kbps | Good intelligibility, low P2P relay cost. |
| In-band FEC | Enabled | Recovers from single-packet loss without retransmit. |
| VAD threshold | 0.02 RMS | Energy-based gate; combined with RNNoise classifier. |
| VAD hold | 300 ms | Trailing silence kept open to avoid clipping word endings. |
| Noise suppression | RNNoise (`nnnoiseless`) | ML-based denoiser; runs in real time on commodity CPU. |
| Echo cancellation | AEC3 (WebRTC port) | Standard echo canceller; required when speakers are not muted. |

## Adaptive jitter buffer

Mouth-to-ear latency budget for the architecture's `<100 ms` target leaves
roughly 40 ms for the jitter buffer after capture, Opus algorithmic delay,
decode, and playback overhead. Beyond 1:1 calls, additional latency must
be spent absorbing per-source desynchronisation in the MCU mix.

```rust
match group_size {
    0..=3 => 40,    // 1:1 and small huddles — tight budget, low loss
    4..=8 => 80,    // medium groups — absorb MCU desync
    _     => 120,   // 9+ — meeting-style, prioritise glitch-free over duplex
}
```

The defaults match industry VoIP norms (Mumble 20–50 ms, Discord ~40 ms,
WebRTC ~50 ms). Adaptive growth on observed loss is the correct long-term
solution; the static-by-group-size approximation is what ships today.

The buffer itself (`jitter::JitterBuffer`) is a `BTreeMap` keyed by
sequence number — packets arriving out of order are reordered, late
arrivals beyond the buffer window are dropped, and the consumer pulls in
order at the configured target latency.

## Transport: `app_message` over a 3-hop Safe route

```rust
SafetySelection::Safe(SafetySpec {
    preferred_route: None,
    hop_count: ANONYMITY_HOP_FLOOR, // 3 — Tor-class, uniform across all paths
    stability: Stability::LowLatency,
    sequencing: Sequencing::NoPreference,
})
```

Voice routes through the same anonymity floor as every other path. It
does **not** use `SafetySelection::Unsafe`:

1. Unsafe routing exposes the sender's real Veilid node identity to the
   first relay. On a vulnerable-user platform that deanonymization is
   unacceptable even when call participants are mutually known — a
   compromised relay (or a single relay at 1 hop) could link a user to
   their network identity and traffic pattern. It is also gated behind
   veilid-core's `footgun` cargo feature, which Rekindle never enables.
2. `LowLatency` stability + `NoPreference` sequencing make voice the
   lowest-latency variant *within* the 3-hop Safe floor. The latency
   cost of the relay hops is the deliberate price of anonymity (see the
   ITU-T G.114 budget in `crates/rekindle-voice/tests/latency_budget.rs`).
3. Each voice packet exposes only that *some* route is sending audio at
   timestamp T. The audio is an RFC 9605 SFrame ciphertext (see
   "Frame encryption" below).

The transport layer (`VoiceTransport`) wraps `routing_context.app_message`
with the route blob from the per-peer registry entry. **Voice frames are
loss-tolerant** — Opus FEC + PLC handle small gaps; retransmission would
add latency without benefit, so failed frame sends are dropped after
warn-level telemetry (W14.4 `VoiceEvent::PacketsDropped` with reason).
**Voice signaling envelopes are NOT loss-tolerant**; their reliability is
D8's call transactions (plan E4.2). The W16 `pending_envelopes` primitive
meant for them was never wired and was deleted (plan E2.3); see
§Signaling and session state below.

`VoiceMode` lets a session prefer ordered delivery (`PreferOrdered`)
when the network conditions warrant it; the default is
`NoPreference`, which Veilid optimises for latency.

## Mutual-aid SFU pattern

For voice channels with more than 4 participants, every speaker sending
to every listener becomes O(N²) bandwidth. Rekindle uses a deterministic
SFU (Selective Forwarding Unit) without a dedicated server:

| Participants | Topology | Outbound packets per speaker |
|--------------|----------|------------------------------|
| ≤ 4 | Full mesh | N − 1 |
| > 4 | Mutual-aid SFU | 1 (to the elected SFU) |

**Today's code is an MCU, not an SFU.** The host is selected
deterministically by `rekindle-voice/src/election.rs`: the participant
whose pseudonym has the lowest XOR distance to `blake3(channel_id)`
wins. Same input → same output → every peer agrees without
coordination, and the role moves when the host leaves. The host
(`mcu_loop.rs`) decodes and mixes the streams, so it is a participant
that hears plaintext (threat model I13).

**Target (plan E4.4):** a forwarding-only relay that never decodes
(D27), elected by maximum advertised uplink with ties broken by minimum
`blake3(scope‖peer)`. Mixing then happens on each listener.

## Signaling and session state

**Wave 13 / Wave 16 supersession**: Voice and call signaling moved off
Veilid `app_call` (synchronous RPC) to fire-and-forget `app_message`.
(The W16 `pending_envelopes` reliability layer planned on top was never
wired and was deleted, plan E2.3; E4.2's call transactions are the
reliability.) Reasons:

- `app_call`'s synchronous RPC layer collapses at ~25 s under network
  churn; a long ring window can lose the call mid-handshake.
- `app_call` traffic shape leaks "this is a call signal" vs DM (a
  privacy regression for vulnerable users).
- Reference architectures (SimpleX, Signal RingRTC, WebRTC standard) all
  use async signaling for the same reasons.

The supersession applies to both 1:1 call signaling
(`CallInvite`/`CallAccept`/`CallDecline`/`CallRinging`/`CallEnd`/
`CallMediaState`) and group call signaling
(`GroupCallOffer`/`GroupCallAccept`/`GroupCallDecline`).
Receiver-side reliability is "voice peer accepted" knowledge via the
`ChatEvent::CallAccept`/`CallConnected` path.

`services/calls/mod.rs` owns the call state machine; `services/calls/
state.rs` defines the per-call status (Outgoing, Incoming, Connecting,
Active); `services/calls/ring_timer.rs` enforces 30 s timeouts on
Dialing and Incoming. Voice transport setup
(`rekindle-voice/src/session/`) only runs after the W14.1 permanent
ingress is in place — no per-call channel construction races.

The Phase 14.q `CallRegistry` trait (defined in
`rekindle_calls::signaling`) is the boundary between the call state
machine and the Tauri shell. `AppState.active_calls: Arc<dyn
CallRegistry>` holds the live registry; the adapter implementation
lives at `src-tauri/src/services/calls_adapter/` and follows the
standard runtime / adapter / pure-logic split documented in
[`services-pattern.md`](services-pattern.md). The trait surface lets
`rekindle-calls` own the lifecycle decisions (ring timeout, accept,
decline, end) while the adapter owns the AppState writes and the
`event_dispatch` emits. Calls are invoked via the IPC commands
`start_dm_call`, `accept_dm_call`, `decline_dm_call`, `end_dm_call`,
`send_call_media_state`, `send_call_reaction`, `mute_caller_temp`,
plus the group-call variants — all registered through `invoke.rs`.

`rekindle-voice/src/signaling/` retains the wire types for community
voice operations (join/leave/handraise/server-mute) which still use a
combination of governance entries and gossip. `session.rs` holds the
per-channel session state in `VoiceSessionMap`. Session state lives in
`AppState.voice_engine` as `VoiceEngineHandle` (engine + transport +
background task join handles).

**Transport mutations are channel-gated.** `VoiceJoin`, `VoiceLeave`,
and `VoiceRoster` envelopes arrive over the community mesh for every
channel (they drive the occupancy UI for all members), but they only
mutate the local `VoiceTransport` peer set — and trigger MCU
election / roster replies — when `voice_engine_bound_to(community,
channel)` matches the active session. A join in a channel we are not
in must never add a peer to our media roster: the transport roster is
the recipient set for both voice frames and directed channel video.

**Active call state is intentionally NOT persisted** — matches Signal
RingRTC and Discord Voice Gateway. Voice transport state (cpal stream
identity, opus encoder state, jitter buffer state,
signing key context) is process-bound and cannot meaningfully resume
across crash. Only Dialing/Incoming envelopes persist (W16.3) so a
30 s ring window survives a quick app restart.

On disconnect or shutdown, `rekindle-voice/src/session/shutdown.rs` joins the
background tasks, drops the cpal streams (which terminates the capture
and playback threads), clears the registry's `voice_channel` field, and
clears `state.voice_packet_tx` + `state.voice_packet_rx_staged` (W15.5).

## Device hot-plug and failure recovery

`AudioCapture` and `AudioPlayback` accept a `device_error_tx: mpsc::Sender<String>`
that fires on cpal `BuildStreamError`, `PlayStreamError`, or stream
termination. The merged receiver is consumed by
`rekindle-voice/src/session/device_monitor.rs`, which:

1. Pauses the affected pipeline (capture or playback).
2. Re-enumerates devices via `crates/rekindle-voice/src/device.rs`.
3. Selects the new system default (or the user's previously chosen
   device if it reappears).
4. Restarts the pipeline.
5. Emits a `VoiceEvent::DeviceChanged` to the frontend so the UI can
   surface the change.

This makes Bluetooth headset switches, USB audio interface unplugs, and
default-device changes survivable without dropping a call.

## Frame encryption (SFrame)

Every voice frame — 1:1 call, community channel, and the MCU host's
mixes — is sealed with SFrame (RFC 9605, cipher suite 0x0005
`AES_256_GCM_SHA512_128`) by `rekindle-voice::media_crypto`, on the
primitive in `rekindle-secrets::sframe`.

- **Sender keys (RFC 9605 §5.1).** Each sender encrypts under its own
  base key: `HKDF-SHA512(scope_secret, "rekindle-voice-sender-key-v1" ‖
  sender_key ‖ tag)`. The secret is the call secret (1:1) or, in a
  community channel, the sender's own media key (below). The receiver derives the key from the
  packet's signed `sender_key`, so a frame opens only as the member who
  signed it, and no shared member ordering is needed (MatrixRTC
  MSC4143 `m.per_member` uses the same per-member model).
- **KID** = 56-bit random per-session sender tag ‖ low 8 bits of the key
  index (0 for calls, the sender key's index for channels). The tag
  makes every session's key fresh, so no key repeats a `(key, nonce)`
  across restarts.
- **Counter.** The CTR lives with the call (`CallState.media_sender`) or
  with each of our channel sender keys (`ChannelSenderKeys`, kept in
  `AppState.voice_sender_keys` for the channel session), so a rebuilt
  transport (route heal, device switch) continues it (§9.1).
- **Metadata.** The packet's `sender_key ‖ sequence ‖ timestamp ‖
  transport_seq` is SFrame metadata (§9.4): a relay cannot splice a
  frame under another sender or position.
- **Replay.** A per-sender, per-KID window over CTR (RFC 3711 §3.3.2
  style, §9.3); failures are counted, never logged per frame (§4.4.4).

The wire packet is `schemas/voice_packet.capnp` behind the one-byte
`'V'` media tag, signed under `rekindle-voice-packet-v2`. The SFrame
plaintext is a VAD level byte followed by the Opus frame.

## Channel media keys (plan C7.20, ADR 0016)

Community call media — voice and video — is not under a MEK. Each
participant keys its own media and hands the key to each participant on
its roster (`rekindle_secrets::media_sender_key`; RFC 9605 §5.1 sender
keys, the MatrixRTC per-member model). No rotator is elected: every
sender owns its own key, so there is nothing to agree on.

- **Key.** A random 32-byte secret per channel session. The key index
  is RFC 9605 §5.1's two-part KID, `generation = index >> 8`,
  `step = index & 0xff`, starting at a random 32-bit generation. Video
  derives its frame key from the secret (`video_frame_key`).
- **Session key (C7.29 G2).** Each participant makes an X25519 key per
  channel session (`media_sender_key::session::SessionKey`, held by the
  keyring) and announces its public half on `VoiceJoin`, `VoiceJoinAck`
  and each `VoiceRosterEntry` (`sessionKey`, `sessionKeySig`), signed by
  its pseudonym over `rekindle-media-session-key-v1 ‖ community ‖
  channel ‖ key`. A key is learned only under that signature
  (`learn_session_key`). The secret is dropped (zeroized) with the
  keyring at `ChannelSenderKeyStore::end`/`clear`, so a recorded
  `VoiceMediaKey` stays closed after the session even if the pseudonym
  is later seized (RFC 9180 §9.1).
- **Distribution.** `ControlPayload::VoiceMediaKey` (@71; request @72,
  ack @73): HPKE Base to the recipient's session key, AAD channel ‖
  recipient ‖ index, plus the sender's pseudonym signature over
  `rekindle-media-key-sig-v1 ‖ channel ‖ recipient ‖ index ‖ sealed`
  (`signature`). It is sent pairwise to the channel roster, acknowledged
  and retried, and opened only by its recipient. Nothing is sealed to a
  participant whose session key we do not hold; its join or ack brings
  one, and it asks with `VoiceMediaKeyRequest` meanwhile.
- **Join (C7.29 G1).** On a join a sender whose key sealed a frame steps
  it forward (`sframe::ratchet`: `HKDF-Expand(HKDF-Extract("", key),
  "SFrame 1.0 Ratchet", 32)`), erases the previous step and sends the
  new step to the joiner only; existing receivers derive it, at most
  `MAX_RATCHET_AHEAD` = 5 steps (RingRTC `MAX_OOO_RATCHETS`). Joins while
  the key is unused coalesce into one step. A join at step 255 starts a
  new generation instead, sent to everyone, and the joiner waits for it.
  The keyring records each participant's admission index, and a
  `VoiceMediaKeyRequest` is answered with keys from that index on
  (`shareable_for`), so a joiner never gets a key that sealed frames
  before it joined. A voice frame's KID carries only the step, so the
  receiver tries each held generation's candidate at that step and keeps
  the one that opens (`receive_candidates`, `confirm`).
- **Leave (C7.29 G3, G4).** When a participant leaves the channel or is
  expired from the roster, we start a new generation with a fresh secret,
  sent to everyone left and used after 5 s. At most one rotation is
  pending (RingRTC): a leave during the delay marks another, started
  when the pending one is applied (`follow_up`), so no index is ever
  bound to two secrets and a leaver hears at most ~10 s after leaving.
  A member leaving the community (kicked, banned, departed;
  `member_departed`, `Departure::LeftCommunity`) always rotates, in a
  stage channel too; only a stage listener leaving on its own does not.
- **Missing key.** A frame under an index we lack sends a
  `VoiceMediaKeyRequest` to its sender (at most once per sender every
  10 s). The sender answers only a requester on its roster.
- **Retention.** Five keys per sender, so frames in flight across a
  rotation still open.
- **Stage channels** never rotate on a listener's voluntary join or
  leave (§10.7: anyone may listen). *Gap (C7.29 G4):* today they do not
  rotate when a listener is kicked or banned either; C7.29 makes every
  community departure rotate.

For 1:1 calls, the call key is X25519 ECDH between fresh per-call
ephemeral keypairs (`rekindle_calls::fresh_keypair`, `derive_call_key`)
with the call ID as HKDF salt, so it is forward-secret. Group calls
(`rekindle-calls/src/group.rs`) still wrap an initiator-made key per
participant; E4.2 moves them onto the sender-owned keyring and deletes
`wrap_call_key`. See [`crates.md`](crates.md) for the `rekindle-calls` crate.

## Where to look

| Concern | File |
|---------|------|
| `VoiceEngine` orchestration | `crates/rekindle-voice/src/lib.rs` |
| Capture (cpal input thread) | `crates/rekindle-voice/src/capture.rs` |
| Playback (cpal output thread) | `crates/rekindle-voice/src/playback.rs` |
| Opus encode/decode | `crates/rekindle-voice/src/codec.rs` |
| RNNoise + AEC3 + VAD | `crates/rekindle-voice/src/audio_processing.rs` |
| Adaptive jitter buffer | `crates/rekindle-voice/src/jitter.rs` |
| Per-participant mixer | `crates/rekindle-voice/src/mixer.rs` |
| Veilid `app_message` transport | `crates/rekindle-voice/src/transport.rs` |
| Device enumeration | `crates/rekindle-voice/src/device.rs` |
| Threading helpers | `crates/rekindle-voice/src/audio_thread.rs` |
| Send loop (capture → encode → transport) | `crates/rekindle-voice/src/send_loop/` |
| Receive loop (transport → jitter → decode) | `crates/rekindle-voice/src/receive_loop/` |
| MCU mixer loop | `crates/rekindle-voice/src/mcu_loop.rs` |
| Voice signaling (join/leave/hand-raise) | `crates/rekindle-voice/src/signaling/` |
| MCU host election (E4.4: forwarding relay) | `crates/rekindle-voice/src/election.rs` |
| Per-channel session map | `crates/rekindle-voice/src/session/` |
| Device hot-plug monitor | `crates/rekindle-voice/src/session/device_monitor.rs` |
| Graceful shutdown | `crates/rekindle-voice/src/session/shutdown.rs` |
| IPC commands | `src-tauri/src/commands/voice.rs` |
| Frontend voice store / panel | `src/stores/voice.store.ts`, `src/components/voice/` |
