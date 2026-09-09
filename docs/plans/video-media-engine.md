# Video media engine — moving codec work behind the daemon

## Why this exists

The event-vocabulary convergence (plan `alignment-completion.md` §4.1)
ended at the video family and could not finish. Every other family
converged because both tracks could express the same facts. Video
cannot, for a structural reason: **the desktop's video encoder lives in
the webview.**

That forces codec-control messages — keyframe requests, bitrate
targets, session config — across the process boundary, and it makes a
CLI or TUI video client impossible regardless of what the event
vocabulary says.

This plan fixes the structure, not the vocabulary.

---

## What is actually true today (verified, not assumed)

| claim | evidence |
|---|---|
| `rekindle-video` has **no codec** | `lib.rs` exports `fragment`, `reassembler`, `pacer`, `policy`, `budget`, `send`/`receive`. Its own header says "Pure logic — no codec FFI". |
| The `VideoCodec` trait **does not exist** | Only a doc comment at `rekindle-video/src/lib.rs:3` mentions it. No `trait VideoCodec` anywhere in the workspace. |
| Encode is split by platform | macOS/Windows: `new VideoEncoder(...)` in `src/components/voice/video_call/video_sender.ts:151` (WebCodecs). Linux: `rekindle-video-capture`, GStreamer `vp9enc`, `pipeline/start.rs:75`. |
| The split is visible in the FIR path | `useVideoCallPanel` branches `ctx.nativeStreamId ? forceNativeKeyframes() : sender.forceKeyframeAll()`. `video_adapter.rs:296` documents it: "Native-owned streams answer keyframe requests in the backend; the event still flows to the frontend, where forceKeyframe is a no-op for stream ids the webview sender doesn't own." |
| Frames cross IPC **compressed** | `video_channels.rs`: `encoded_payload_b64` / `payload_b64`. Even the Linux self-preview is JPEG (`PreviewFrame { jpeg: Vec<u8> }`, 320×180), not raw. |
| The daemon has **no video at all** | `rekindle-node` and `rekindle-cli` have zero video dependencies and zero video references in `src/`. |
| The daemon **drops every video payload** | `into_event_control.rs:319-325`: `VideoFragment`, `VideoParityFragment`, `FrameAck`, `KeyframeRequest`, `BandwidthEstimate`, `MediaCapabilities`, `TopologyChange` all fall into `=> return None`. |
| The daemon IPC has **no frame transport** | `BusPayload` is `Request` / `Response(Vec<u8>)` / `Event(SubscriptionEvent)`. Nothing carries media. |
| Three video events are emitted and never consumed | `videoFrameAck`, `videoBandwidthEstimate`, `videoMediaCapabilities` appear only in `community_video_events.ts` as type declarations. No handler reads them. |

---

## Research — all three sources, including the one that disagrees

### Our own architecture

- Tier 1 is *"shared type definitions, zero logic, zero I/O"*
  (`crates-tier-detail.md`). It **already holds video vocabulary**:
  `Codec` and `ScalabilityMode` live in `rekindle-types/src/video.rs`.
- `event-dispatch.md` already separates the two concerns deliberately:
  video **frames** bypass the event queue via Tauri `Channel<T>`
  ("28 KB or more and arrive at video frame rate; routing through the
  single mpsc would bottleneck"), video **control** rides the event
  channel.
- The `Codec` enum's own doc records why the set is what it is —
  and it is reasoning *about the webview*: "Apple WebKit guarantees
  H.264 hardware encode but not VP9; WebKitGTK guarantees VP8/VP9 via
  libvpx but H.264 only via optional GStreamer plugins."

  That constraint is a consequence of where the encoder lives. Move the
  encoder and the constraint changes.

### Veilid (vendored `veilid-core-0.5.7`, not the web)

- `MAX_APP_MESSAGE_MESSAGE_LEN = 32768`
  (`rpc_processor/coders/operations/operation_app_message.rs:3`). The
  only hard constraint here, and already respected — video fragments
  cap at 4 KiB.
- Veilid accounts bandwidth and latency internally
  (`TransferStatsAccounting`, `LatencyStatsAccounting`, the
  `upload_bandwidth_gate` / `download_bandwidth_gate` semaphores in
  `storage_manager/mod.rs`) and exposes aggregate node stats — but
  **no per-peer receiver-side media feedback**. `routing_context.rs`
  surfaces none of it.
- Veilid's relay is a **transport-level NAT-traversal relay node**
  chosen by the routing table, not an application media SFU. Our
  `TopologyChange.relay_host_pseudonym` (mutual-aid SFU) is our own
  concept.

**Conclusion: Veilid neither provides nor constrains this layer**, the
32 KiB cap aside. Same finding the gossip TTL/fan-out work reached.

### Authoritative external

- [RFC 7742](https://www.rfc-editor.org/rfc/rfc7742.html) — VP8 and
  H.264 Constrained Baseline are mandatory-to-implement for WebRTC
  browsers **and non-browsers**. VP9 is not. Our `Codec` enum already
  carries exactly VP8 / VP9 / H.264.
- [RFC 5104](https://www.rfc-editor.org/rfc/rfc5104) — FIR ("Full Intra
  Request") and TMMBR (bitrate ceiling) are **codec control messages
  between endpoints**. They are protocol vocabulary, not UI state.
- **[Jami](https://docs.jami.net/en_US/developer/jami-concepts/calls.html)
  — the source that disagrees, and the closest architectural match.**
  Jami is a daemon with thin clients, exactly the shape we want. Its
  codec negotiation happens **inside the daemon**; the client API is
  `answerCallWithMedia()`; keyframe requests and RTCP live inside the
  media stream, never on the client API.

**Reconciliation.** Jami's containment works because Jami's media engine
is in its daemon. Ours is not, so the conclusion does not transfer —
but the *principle* does, and it is the design rule for this plan:

> Only what a frontend must **act on** should cross the boundary.
> Feedback plumbing stays inside whichever process owns the encoder.

- libvpx builds on Linux, macOS and Windows, with several Rust bindings
  (`vpx-rs`, `vpx-encode`, `env-libvpx-sys`). AV1 (`rav1e`) is pure Rust
  but is neither in our codec set nor MTI, and realtime AV1 encode is a
  different performance class.

---

## The constraint that shapes the design

**Encode can move to the backend. Decode largely cannot — for a webview
frontend.**

Raw frames cannot cross the IPC cheaply. 1280×720 YUV420 is
1280 × 720 × 1.5 = **1.38 MB per frame**; at 15 fps that is **~20 MB/s
per peer**. There is no path in Tauri to hand a webview a raw frame
without a copy.

The project already decided this, twice: remote frames cross as
`encoded_payload_b64` and are decoded by WebCodecs in the webview, and
even the 320×180 Linux self-preview is JPEG rather than raw.

So the target is **not** "move all media into the daemon". It is:

- **Encode** → backend, one path, all platforms.
- **Compressed frames** → the transport the daemon already owns, handed
  to whichever frontend asked for them.
- **Decode/display** → the consumer's business. A webview uses
  WebCodecs. A TUI could pipe to `ffplay`. A headless recorder writes a
  file. None of that is the daemon's concern.

That is the same split the daemon already uses for audio, where
`rekindle-voice` owns Opus + VAD + jitter buffer + mixer and no
frontend sees a codec.

---

## Plan

### Step 0 — stop emitting what nothing consumes

Delete the frontend emission of `VideoFrameAck`,
`VideoBandwidthEstimate` and `VideoMediaCapabilities`.

- All three are declared in `community_video_events.ts` and read by no
  handler.
- The backend already consumes them itself: `video_adapter.rs:185-200`
  calls `apply_bitrate_feedback(...)` for `FrameAck | BandwidthEstimate`,
  and that runs in `emit_event` **before** `map_video_event` builds the
  frontend payload — so removing the emit cannot break the feedback
  loop. Verified by reading both call sites.
- Backed by all three sources: Jami says feedback is not client-facing;
  `event-dispatch.md` says everything on the queue should have a
  consumer; and it is measurable waste today.

This is a deletion, and it is correct whether or not the rest lands.

### Step 1 — the daemon decodes the video wire

The blocker for "CLI is the driver" is not the engine, it is that
`into_event_control.rs` returns `None` for every video payload.

Produce Tier 1 events from `KeyframeRequest`, `BandwidthEstimate`,
`MediaCapabilities`, `TopologyChange`. Fragments are not events — see
Step 3.

While here: the same `=> return None` arm also drops `StageUpdate`,
`SpeakRequest`, `SpeakResponse` and `SoundboardPlay`, which now **have**
Tier 1 variants (added in the voice-signalling convergence) that the
daemon never produces. Same one-line-per-variant fix, same file.

### Step 2 — extract the encoder behind a trait

Create the `VideoCodec` trait the architecture has been claiming exists.

```
crates/rekindle-video/src/codec.rs   (Tier 7, no FFI — trait only)

pub trait VideoEncoder: Send {
    fn configure(&mut self, cfg: &EncoderConstraints) -> Result<(), VideoError>;
    fn encode(&mut self, frame: RawFrame) -> Result<Option<EncodedFrame>, VideoError>;
    fn force_keyframe(&mut self);
    fn set_bitrate(&mut self, kbps: u32);
    fn codec(&self) -> Codec;
}
```

Implementations:

| impl | where | status |
|---|---|---|
| GStreamer | `rekindle-video-capture` | **exists** — camera → `vp9enc` → appsink. Refactor to satisfy the trait. |
| libvpx | new, all platforms | VP8 + VP9, the RFC 7742 floor. The cross-platform path. |
| WebCodecs | webview | **demoted to a fallback**, not the primary. |

`set_bitrate` and `force_keyframe` are the whole point: they are what
`VideoBitrateTarget` and `VideoKeyframeRequest` currently cross the
boundary to do.

### Step 3 — a frame transport for the daemon

The desktop routes frames through Tauri `Channel<T>`, deliberately
outside the event queue. The daemon has no equivalent — `BusPayload`
carries only `Request`, `Response(Vec<u8>)` and
`Event(SubscriptionEvent)`.

Adding frames to `Event` would be wrong for the reason
`event-dispatch.md` already gives: it would put frame-rate traffic
through the single ordered queue. The daemon needs a `BusPayload::Media`
variant on its own path, or a separate socket.

**This is the step with a real open question**, and it should not be
guessed at. Options, with the trade-off stated rather than resolved:

- a `BusPayload::Frame(Vec<u8>)` variant with its own backpressure
- a second unix socket per media session
- a shared-memory ring, which is what Jami does on Linux (SHM sink)

### Step 4 — the vocabulary follows

Once the encoder is backend-side:

- `KeyframeRequest` and `BitrateTarget` are consumed internally, and
  stop crossing except to the WebCodecs fallback.
- `SessionVideoConfig`'s encoder half becomes internal; only the decoder
  half needs to reach a consumer.
- What remains frontend-facing is what a *viewer* needs:
  `TopologyChange`, `CodecIncompatible`, `NativeVideoError`.

That is Jami's shape — reached deliberately, rather than by the accident
of WebCodecs being where the encoder happened to be.

Only then do the pure-data structs (`SessionVideoConfig`,
`EncoderConstraints`, `DecoderConstraints`, `MediaCapabilities`,
`BandwidthEstimate`) move from `rekindle-video` (Tier 7) down to
`rekindle-types` (Tier 1), the way `Codec` and `ScalabilityMode` already
did — a Tier-1 event family cannot depend upward on Tier 7.

---

## Sequencing and risk

Step 0 is a deletion and lands alone. Step 1 is confined to one match
arm in one file and unblocks CLI video observation without touching the
engine. Steps 2–4 are the rearchitecture and are gated on Step 3's open
question.

The honest risk in Step 2 is the libvpx C dependency reaching macOS and
Windows Tauri packaging, where today only Linux carries a native media
dependency. That should be proven with a build before the trait work,
not after.
