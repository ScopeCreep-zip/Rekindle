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

### Step 3 — a frame transport for the daemon — **RESOLVED**

**Decision: a new `BusPayload` variant on the existing Noise bus. Not
shared memory, not a second socket.**

This reverses the option this plan originally leaned toward. Jami uses
shared memory, so SHM looked like the answer; checking why it uses SHM
is what changed the conclusion.

**Our architecture.** The IPC bus is already
`Noise_IK_25519_ChaChaPoly_BLAKE2s` over a Unix socket, with UCred
mixed into the prologue explicitly as anti-confused-deputy protection
(`daemon-cli.md`). And it **already chunks**: `ipc/noise.rs` sets
`MAX_NOISE_PLAINTEXT = 65519` and carries application frames up to
`MAX_FRAME_SIZE = 16 MiB` behind a chunk-count header. A 200 KB
keyframe needs no new framing code at all.

A shared-memory segment would sit *outside* that boundary — no
authentication, no UCred binding, no forward secrecy. It would trade
away the security property the bus was built for.

**Veilid.** Not in this path — daemon↔client IPC is ours. And it offers
no pattern to borrow: `routing_context.rs` exposes `app_call`,
`app_message` and DHT record operations. Message-oriented, no stream
API. No constraint either way.

**External, including the source that pointed the other way.**
[Jami's SHM sink](https://dl.jami.net/doxygen/daemon/videomanager__interface_8h_source.html)
is a **raw-frame** path: double-buffered, "avoids copying raw frame
data multiple times between processes", feeding a native client that
blits decoded frames. That is zero-copy for *decoded* video.

We do not have that problem, by our own design. We ship **compressed**
frames, and our own budget bounds them: `VIDEO_MAX_KBPS = 600`, i.e.
**75 KB/s per stream** (`rekindle-video/src/budget.rs`). Eight
concurrent streams is 600 KB/s.

ChaCha20-Poly1305 runs at
[~1.15 GB/s on 8 KB inputs with AVX2](https://github.com/aead/chacha20poly1305),
~4.2 GB/s on an M3 Pro. 600 KB/s against 1.15 GB/s is **≈0.05 % of one
core**. Shared memory would be optimising a cost that does not exist,
and paying for it with the security boundary.

**What still has to be built**, because "reuse the bus" is not "reuse
the event stream":

- Frames must **not** ride `BusPayload::Event(SubscriptionEvent)`. That
  is the ordered, deduped, journaled stream that `event-dispatch.md`
  deliberately keeps frame-rate traffic out of, for the same reason the
  desktop routes frames through a Tauri `Channel<T>` instead of the
  event queue.
- So: a sibling `BusPayload::Media` variant, bypassing the event
  router's dedup and journal, with its own bounded queue and a
  drop-oldest policy — a late video frame is worthless, unlike an event.
- The 100-token/second rate limit in `ipc/server/routing.rs` is applied
  to **inbound** frames from a client. It does not throttle daemon→client
  media, but it does bound a client that sends frames, which matters if a
  frontend ever encodes.

**The one real trade-off, stated rather than hidden:** frames share the
socket with requests and responses, so a large frame briefly delays a
control message. It is bounded by frame size — a 200 KB keyframe is
four 64 KB chunks, sub-millisecond at these rates — and is the price of
staying inside the authenticated channel. If it ever bites, the fix is a
second Noise session on the same socket, not shared memory.

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

## Quality parity with Discord and comparable services

Requirement added after the transport decision. It splits cleanly into
two problems with very different answers, and conflating them would
produce a wrong plan.

### Where we actually are

| | ours | Discord |
|---|---|---|
| voice codec | Opus 48 kHz **mono, 32 kbps** (`voice/src/codec.rs:61`) | Opus 48 kHz, **64 kbps default**, 96/128/256/384 with boost, adapts to 8 kbps under congestion |
| video | **854×480 @ 15 fps**, VP9 (`MediaCapabilities::interim_default`) | 720p30 standard, higher with Nitro |
| video bitrate | 350 kbps start, 600 kbps ceiling (`budget.rs`) | ~2.5 Mbps class |
| media path | **3-hop anonymous Veilid route**, "never Unsafe" (`voice/src/transport.rs:142`) | direct UDP / their SFU, no anonymity |

### Voice — parity is achievable and cheap

Discord's default is 64 kbps Opus; we run 32 kbps mono. The comment at
`codec.rs:57` gives the reason — "reduces P2P relay load" — which was
sound, but the cost is small in context: +32 kbps is ~9 % of the
350 kbps *video* start budget, and `OpusCodec::set_bitrate` already
exists for adaptive use.

This is independent of the engine and can be done on its own. Stereo is
a separate question (2× the bitrate for a marginal gain on speech;
Discord defaults mono for voice channels too).

### Video — two separable problems

**1. Quality per bit — the engine fixes this, and this is the real
argument for doing it.**

Our own dependency comment already says what the webview costs us:

> "Native camera capture + vp8enc realtime CBR — replaces the webview
> encode path on Linux (**WebKitGTK's VP9 rides libvpx GOOD-deadline,
> no true CBR**)."
> — `src-tauri/Cargo.toml`

A realtime call wants `deadline=REALTIME` and `end-usage=cbr`, which is
exactly what the GStreamer pipeline sets and what WebCodecs does not
give us. At 350–600 kbps a quality-deadline non-CBR encoder overshoots
and undershoots the target and looks materially worse than a properly
rate-controlled one at the same bitrate.

So the engine is not only architectural tidiness — **it is the quality
fix**, and it applies to macOS and Windows, which today have no native
path at all. That is parity work that costs no extra bandwidth.

**2. Absolute bitrate — bounded by anonymity, not by the codec.**

The 600 kbps ceiling is not arbitrary. `budget.rs` records the
measurement: sized for "sustained 480p15 over a 3-hop Veilid Safe
route", after 1200 kbps "saturated the egress the unpaced voice stream
shares, starving audio".

Discord does not pay that cost — it runs direct UDP or its own SFU with
no per-frame anonymity. Published Tor research puts three-hop onion
routing at roughly +83 ms latency with throughput constrained by relay
congestion, and describes realtime HD over standard three-hop circuits
as challenging.

**So Discord-class resolution and per-frame anonymity are in tension,
and that is a product decision rather than an engineering one.** The
honest options, none of which should be chosen silently:

- **Keep 3-hop, take the quality win from (1).** Better 480p, possibly
  540p/24, at the same bitrate. No privacy change.
- **Mutual-aid SFU.** Relay through one community peer instead of a
  3-hop onion per pair — the `TopologyChange.relay_host_pseudonym`
  machinery already exists. Fewer hops, more throughput, and the relay
  peer learns who is in the call.
- **Lower hop count for media only.** Directly contradicts
  `transport.rs:143` — "anonymous on every voice frame, never Unsafe" —
  so it would be reversing a stated commitment, not tuning a constant.

My recommendation: do (1), which is free, and treat the resolution gap
as an explicit, documented product tradeoff rather than a defect. Claim
"480p30 anonymous" rather than "720p30 like Discord", because the
second is not reachable without giving something up.

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
