# Voice Architecture

The voice pipeline carries low-latency audio between peers in 1:1 calls,
DM/group-DM calls, and community voice channels. It is built on Veilid's
`app_message` primitive with `SafetySelection::Unsafe` for sub-50 ms
delivery, and on `cpal` for cross-platform audio I/O.

The implementation lives in two places:

- **`crates/rekindle-voice/`** — pure-logic audio pipeline (capture,
  encode, transport, jitter, mix, playback).
- **`src-tauri/src/services/voice/`** — the Tauri-side orchestration
  (send/receive/MCU loops, signaling, election, device monitor, session
  state, shutdown).

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
                    │  VoiceTransport (send)  │  Veilid app_message, Unsafe routing
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
| Tokio task: send loop | `services/voice/send_loop.rs` | Drains capture, runs `AudioProcessor`, encodes, transports |
| Tokio task: receive loop | `services/voice/receive_loop.rs` | Reads `VoicePacket`s from dispatch, jitters, decodes |
| Tokio task: MCU loop | `services/voice/mcu_loop.rs` | Mutual-aid SFU mixer for >4-participant calls |
| Tokio task: device monitor | `services/voice/device_monitor.rs` | Watches `device_error_rx` for hot-plug / device-failed events |

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

## Transport: `app_message` with `Unsafe` routing

```rust
SafetySelection::Unsafe(Sequencing::NoPreference)
```

Voice packets bypass safety routing for low latency. This is acceptable
because:

1. Voice channel participants are mutually known — the privacy property
   that safety routes provide (sender anonymity) is irrelevant when
   participants already see each other in the channel.
2. Each voice packet exposes only that *some* peer is sending audio at
   timestamp T. The actual audio is encrypted with the channel MEK
   (community voice) or the X25519-derived call key (1:1, see
   [`rekindle-calls`](crates.md#rekindle-calls)).

The transport layer (`VoiceTransport`) wraps `routing_context.app_message`
with the route blob from the per-peer registry entry. **Voice frames are
loss-tolerant** — Opus FEC + PLC handle small gaps; retransmission would
add latency without benefit, so failed frame sends are dropped after
warn-level telemetry (W14.4 `VoiceEvent::PacketsDropped` with reason).
**Voice signaling envelopes are NOT loss-tolerant** — they flow through
the W16 `pending_envelopes` reliability primitive (per-recipient seq_ack,
receiver dedup, route-aware retry up to 5 min, crash recovery for
Dialing/Incoming states); see §Signaling and session state below.

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

The SFU is selected deterministically by `services/voice/election.rs`:
the online voice participant with the lowest `blake3(channel_id || own_pseudonym)`
hash wins. Same input → same output → every peer agrees without
coordination. The SFU role transfers automatically when the elected
participant leaves; the new lowest-hash member takes over.

Encoding stays at the speaker — the SFU does not transcode. It only fans
out frames to the other listeners. Mixing happens locally on each
listener via `AudioMixer`, summing per-participant streams with
per-source gain.

## Signaling and session state

**Wave 13 / Wave 16 supersession**: Voice and call signaling moved off
Veilid `app_call` (synchronous RPC) to fire-and-forget `app_message`,
layered with the W16 `pending_envelopes` reliability primitive
(per-recipient seq_ack, receiver dedup, durable retry up to 5 min,
crash-survives recovery). Reasons:

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
`ChatEvent::CallAccept`/`CallConnected` path; senders learn delivery
outcomes via `ChatEvent::EnvelopeDelivered`/`EnvelopeDeliveryFailed`.

`services/calls/mod.rs` owns the call state machine; `services/calls/
state.rs` defines the per-call status (Outgoing, Incoming, Connecting,
Active); `services/calls/ring_timer.rs` enforces 30 s timeouts on
Dialing and Incoming. Voice transport setup
(`services/voice/session.rs`) only runs after the W14.1 permanent
ingress is in place — no per-call channel construction races.

`services/voice/signaling.rs` retains the wire types for community
voice operations (join/leave/handraise/server-mute) which still use a
combination of governance entries and gossip. `session.rs` holds the
per-channel session state in `VoiceSessionMap`. Session state lives in
`AppState.voice_engine` as `VoiceEngineHandle` (engine + transport +
background task join handles).

**Active call state is intentionally NOT persisted** — matches Signal
RingRTC and Discord Voice Gateway. Voice transport state (cpal stream
identity, opus encoder state, AEAD nonce counters, jitter buffer state,
signing key context) is process-bound and cannot meaningfully resume
across crash. Only Dialing/Incoming envelopes persist (W16.3) so a
30 s ring window survives a quick app restart.

On disconnect or shutdown, `services/voice/shutdown.rs` joins the
background tasks, drops the cpal streams (which terminates the capture
and playback threads), clears the registry's `voice_channel` field, and
clears `state.voice_packet_tx` + `state.voice_packet_rx_staged` (W15.5).

## Device hot-plug and failure recovery

`AudioCapture` and `AudioPlayback` accept a `device_error_tx: mpsc::Sender<String>`
that fires on cpal `BuildStreamError`, `PlayStreamError`, or stream
termination. The merged receiver is consumed by
`services/voice/device_monitor.rs`, which:

1. Pauses the affected pipeline (capture or playback).
2. Re-enumerates devices via `crates/rekindle-voice/src/device.rs`.
3. Selects the new system default (or the user's previously chosen
   device if it reappears).
4. Restarts the pipeline.
5. Emits a `VoiceEvent::DeviceChanged` to the frontend so the UI can
   surface the change.

This makes Bluetooth headset switches, USB audio interface unplugs, and
default-device changes survivable without dropping a call.

## Voice MEK rotation

For community voice channels, the MEK rotates on every join and every
leave — providing strong forward and backward secrecy for live
conversations. A late joiner cannot decrypt earlier voice packets; a
departing member cannot decrypt subsequent packets. See
[`communities.md` §5](communities.md#5-mek-lifecycle-peer-to-peer-no-vault)
for the rotator selection and cascade fallback protocols.

For 1:1 and DM/group-DM calls, the call key is derived deterministically
via X25519 ECDH over the participants' identity keys plus the call ID as
HKDF salt. See [`crates.md`](crates.md) for the `rekindle-calls` crate.

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
| Send loop (capture → encode → transport) | `src-tauri/src/services/voice/send_loop.rs` |
| Receive loop (transport → jitter → decode) | `src-tauri/src/services/voice/receive_loop.rs` |
| MCU mixer loop | `src-tauri/src/services/voice/mcu_loop.rs` |
| Voice signaling (join/leave/hand-raise) | `src-tauri/src/services/voice/signaling.rs` |
| SFU rotator election | `src-tauri/src/services/voice/election.rs` |
| Per-channel session map | `src-tauri/src/services/voice/session.rs` |
| Device hot-plug monitor | `src-tauri/src/services/voice/device_monitor.rs` |
| Graceful shutdown | `src-tauri/src/services/voice/shutdown.rs` |
| IPC commands | `src-tauri/src/commands/voice.rs` |
| Frontend voice store / panel | `src/stores/voice.store.ts`, `src/components/voice/` |
