# 0015 — Media engine: one Rust engine built from ported production components, not libwebrtc `Call` and not re-derived loops

- **Status:** Accepted (owner ruling Q5, 2026-10-07: Option F)
- **Date:** 2026-10-07
- **Plan step:** E4.3.0 part 2, research spike for ruling Q5
  (`.claude/plans/standards-remediation/00-integration-plan.md` §0 table and E4.3)
- **Builds on:** `.claude/plans/architecture-audit/research/r6-media-engines.md` (r6) §0, §1, §2, §7,
  §8 and §9 R-BW, R-AU and R-VI. This draft does not repeat r6. Every source it read is listed in
  `.claude/plans/standards-remediation/evidence/e4-3-0-q5-spike.md`.
- **Related:** ADR 0010 (the daemon owns media), ADR 0012 and ruling Q4 (media keys; this ADR does
  not change them), ADR 0014 (Veilid boundary; the engine never links veilid-core).

## Context and problem statement

E4.3 requires the following engines whichever engine choice is made (plan E4.3, D27):

- one estimator, one allocator and one pacer per peer route, with audio at the head of the pacer;
- GCC-class congestion control on per-packet arrival feedback, reacting within max(1 s, 3 RTT);
- a NetEQ-class audio playout buffer (p95 target from a forgetting histogram, time-stretch, drift
  handling, PLC merge);
- A/V sync under 80 ms, and keyframe requests after max(200 ms, RTT), repeated no faster than every
  200 ms.

Rekindle derives each of these by hand today, one at a time, and keeps rebuilding them:

- **Two estimators on one route.** `crates/rekindle-video/src/budget.rs:42` says "ours are separate
  estimators, so this is the explicit hand-off". That file has an AIMD loop on `FrameAck` loss. The
  voice side has a separate 5 s receiver-report ladder (`crates/rekindle-voice/src/send_loop/quality.rs`).
- **Voice outside the pacer.** `crates/rekindle-video/src/pacer.rs:10-13`: "voice frames never enter
  this queue".
- **The jitter buffer is clamped to 40–120 ms.** It moves in whole 20 ms frames
  (`crates/rekindle-voice/src/jitter/mod.rs:17-22`). The uncommitted `jitter/accelerate.rs` (plan
  C7.23) is a new partial re-derivation of NetEQ's accelerate path, and it works "in whole 20 ms Opus
  frames" (`accelerate.rs:18-21`).
- **The frontend runs its own policy.** It has a second playout buffer
  (`src/components/voice/video_call/playout_buffer.ts`) and a bitrate ladder
  (`video_sender.ts`, `LADDER_*`).
- **The churn shows in git.** 71 commits touch `rekindle-voice`, `rekindle-video` and
  `rekindle-media-stats`. `jitter.rs` alone has 11, `pacer.rs` 8, `budget.rs` 6, and
  `src/components/voice/video_call/` 27. r6 §8 names the rebuild commits (`3ac28f75`, `bf831b68`,
  `6c20eda5`, `1437d5e7`, `b45783f1`).

Ruling Q5 asks whether to keep hand-building this engine or to reuse a production engine. Discord's
native clients are the precedent: they drive libwebrtc `webrtc::Call` with no SDP and no ICE, over
their own `webrtc::Transport`, with their own encryption (r6 §8; `raw/r6/blogs/discord-2.5m-voice.txt:164-169`).

### Constraints any answer must meet

- **Transport.** Veilid `app_message` carries at most 32768 B (`veilid-core/.../operation_app_message.rs:3`).
  Its delivery semantics, per r6 §6:
  - it is a fire-and-forget statement, with no ordering and no delivery guarantee;
  - it goes over a 3-hop safety route plus the peer's private route;
  - each hop segments it into 1272-byte datagrams (`veilid-tools/src/assembly_buffer.rs:20`,
    `FRAGMENT_LEN = 1280 - HEADER_LEN`).

  Nothing reports delivery, so every rate decision must come from receiver feedback.
- **E2EE.** It is SFrame (RFC 9605) with sender keys, already implemented in Rust
  (`crates/rekindle-voice/src/media_crypto.rs`, `rekindle_secrets::sframe`), plus per-packet signatures.
- **Licence.** Rekindle is MIT (`LICENSE`).
- **Memory safety.** `docs/security/cisa-secure-by-design.md` §1 marks memory safety "met". It lists
  `rekindle-video`, `rekindle-node` and `rekindle-transport` among the `#![forbid(unsafe_code)]`
  crates. ADR 0010 makes `rekindle-node` (the daemon) the process that will host media (E4.4).
- **Platforms.** macOS arm64, Linux x86_64 (Pop!_OS, Mac/Pop asymmetric WebCodecs encoder sets) and
  Windows, with the dev shell from the Nix flake (`flake.nix` already provides libopus and GStreamer).
- **The threat model.** It is written for vulnerable users. Media packets come from untrusted peers
  and are parsed in the daemon.

## Decision drivers

1. **Faithfulness to production behaviour.** Each engine should behave like the one in libwebrtc,
   which r6 §2 already measured constant by constant. Approximations that drift should not be
   possible.
2. **One owner per decision on Veilid's transport.** This means no ICE, DTLS or SRTP layer that
   duplicates what Veilid routes and SFrame already do. r6 §8 says to drop or replace those layers.
3. **Memory-safe packet parsing in the daemon.** CVE-2023-7024 was a heap overflow in WebRTC. It was
   exploited in the wild and added to CISA KEV on 2024-01-02 (NVD).
4. **Stable, supported API surface, and a build that the Nix flake and CI can reproduce on all three
   OSes.**
5. **Licence compatibility with MIT.** AGPL code (RingRTC, Signal Calling Service) is reference
   behaviour only.
6. **Keep the work Rekindle owns** (SFrame, signatures, the `app_message` framing, relay topology and
   the WebCodecs codec layer) and delete the hand-derived loops.

## Considered options

### Option A — libwebrtc `webrtc::Call` over a custom transport (the Discord model)

| Aspect | Finding |
|---|---|
| Licence | BSD-3-Clause plus Google's patent grant (`raw/r6/webrtc/libwebrtc/LICENSE`, `PATENTS`). Compatible with MIT; binary notices required. |
| Engine coverage | Everything: GoogCC, `BitrateAllocator`, pacer, NetEQ, video timing, keyframe policy, A/V sync, quality scaler (r6 §1 M1–M15). Encoded-frame E2EE hooks through `api/frame_transformer_interface.h`. |
| API stability | `call/` is **not** in libwebrtc's public API. `native-api.md` lists only `api/` plus legacy directories, and "in the directories not listed … incompatible changes may happen at any time, and are not announced". `call/call.h:56,73` (`class Call`, `Call::Create(CallConfig)`) sits outside that list. |
| Rust bindings | None exist for `Call`. LiveKit `webrtc-sys` 0.3.48 (Apache-2.0, cxx) wraps only the PeerConnection API; grepping its tree for `webrtc::Call`, `call/call.h` and `Call::Create` finds nothing. Rekindle would write and maintain its own C++/cxx bridge. |
| Builds | LiveKit's prebuilt `libwebrtc.a` (m150, `webrtc-sdk/webrtc@m150_release` with 14 patches) is downloaded **at build time** from GitHub releases (`webrtc-sys/build/src/lib.rs:31,101-106`). The zips are 277 MB (mac-arm64), 156 MB (linux-x64) and 114 MB (win-x64), per the GitHub API for tag `webrtc-89d790b`. The Linux archive ships a hermetic libc++ as "an ABI contract". LiveKit's own build script records a std::span ABI mismatch that "turn[ed] a 14-byte DataChannel::Send into new uint8_t[93TB]" (`build_linux.sh:120-122`). Building from source needs depot_tools, gclient and GN, which the Nix sandbox does not provide [lead: no nixpkgs derivation checked]. |
| Over Veilid | Possible in principle: implement `webrtc::Transport`, feed `Call::Receiver()`, and replace SRTP with SFrame through frame transformers. But `Call` also wants to own the codecs and the ADM, so cpal capture, the `aec3` port and WebCodecs video would all have to be bridged or replaced. |
| Risk | It puts a large C++ packet-parsing surface inside the `forbid(unsafe_code)` daemon, against driver 3. It also depends on an unannounced-churn internal API, a third-party fork plus its patches and per-platform binary blobs. |

### Option B — str0m 0.24.1 (Sans-I/O Rust WebRTC, MIT OR Apache-2.0)

| Aspect | Finding |
|---|---|
| Coverage | Pacer, TWCC, a GoogCC BWE ported from libwebrtc (trendline, loss, probe controller, ALR, link capacity; `src/bwe/mod.rs:1-13`), NACK, simulcast, packetisation. **No adaptive jitter buffer**, no capture, encode or decode (README feature table, `:548-566`). |
| Over Veilid | Media cannot flow without DTLS-derived SRTP keys. `session.rs:385` ("Do not make unsendable media … ready before DTLS supplies keys"), `:973` and `:1058` gate on `ready_for_srtp()`. `DirectApi` (`src/change/direct.rs`) skips SDP, not ICE or DTLS. Using the crate means running ICE and a DTLS handshake over multi-hop routes: extra round trips before first media (against R-SES5) and a second encryption layer under SFrame. |
| Reuse as a library | The BWE and pacer are `pub(crate)` (`src/lib.rs:812-819`). Only event types are public (`src/bwe/api.rs`). Using them without the session means copying the code. |
| Builds | Pure Rust with selectable crypto back ends. Fine on all three OSes. |

### Option C — continue hand-deriving each engine (status quo)

Licence and builds are trivially fine. It fails driver 1: the history above is the cost, and the
current jitter work is still a partial NetEQ that is not NetEQ.

### Option D — webrtc-rs `rtc` / `rtc-interceptor` 0.21.0 (MIT/Apache-2.0) as the control engine

| Aspect | Finding |
|---|---|
| Coverage | A public, Sans-I/O, transport-agnostic interceptor crate. Its only dependencies are `rtc-rtp`, `rtc-rtcp`, `rtc-shared` and `sansio`; no ICE, DTLS or crypto. It provides GCC following draft-ietf-rmcat-gcc-02: Kalman filter §5.3 (`src/gcc/kalman.rs`), loss controller §5.5 (`src/gcc/loss.rs`), adaptive threshold and AIMD. It also has TWCC and RFC 8888 recorders, a token-bucket pacer, FlexFEC, NACK and an interval-PLI generator. Published 2026-09-19 (crates.io). |
| Gaps | No probing or ALR, which R-BW5 needs (grep of `src/gcc` finds no probe controller). The pacer is a token bucket with no priority classes (`src/pacing/pacer.rs`). The jitter buffer has a fixed time depth (`DEFAULT_DEPTH` 120 ms, `src/jitterbuffer/receiver.rs`) and is not NetEQ-class. The GCC is a port of Pion's draft-level GCC, not libwebrtc's production GoogCC. It is a weeks-old release line. |

### Option E — a NetEQ port: which one

- **`neteq` crate 0.9.1** (videocall-rs, MIT OR Apache-2.0). It is "NetEQ-inspired": its merge is a
  stub (`src/neteq.rs:880-883`, "Simplified merge - just normal decode for now"), comfort noise is
  scaled white noise (`:885-891`) and the quantile is 0.97, not libwebrtc's 0.95. Its default
  features pull in axum, tokio, cpal and clap. **Rejected** as the engine. It is useful as a
  cross-check only.
- **FFI to libwebrtc NetEQ.** `api/neteq` is public API, but using it means carrying the whole of
  Option A's build for one module. **Rejected.**
- **Faithful Rust port of libwebrtc NetEQ** (BSD-3-Clause). The scope is
  `modules/audio_coding/neteq/` minus DTMF, RED splitter and UMA logging: 11,295 lines of `.cc`/`.h`,
  plus 20 `WebRtcSpl_*` helpers from `common_audio`. Unit tests ship beside each file
  (`delay_manager_unittest.cc`, `decision_logic_unittest.cc`, `merge_unittest.cc`,
  `time_stretch_unittest.cc` and others). **Chosen** (see below).

### Option F — Rekindle-owned Rust engine assembled from vendored and ported production components

Described in the decision below: it combines str0m's GoogCC and pacer (Option B's engine, without
its session), a faithful NetEQ port (Option E's third variant) and libwebrtc's video timing and sync,
behind one allocator that Rekindle owns. rtc-interceptor (Option D) is not used, because it has no
probing or ALR and no priority pacer. Its Kalman tests remain a cross-check.

## Decision outcome (proposed)

**Rekindle keeps its own media engine, in Rust, inside the daemon, but stops deriving its control
loops. Each engine becomes a faithful port or vendored copy of a named production component, held to
that component's own tests and constants. libwebrtc `Call` and the str0m session are not adopted.**

Component by component:

| Engine (r6 §1) | Source | Licence | Lands in |
|---|---|---|---|
| GoogCC estimator (trendline delay, loss, acked rate, probe controller, ALR, link capacity) and leaky-bucket pacer with probe clusters | **Vendored from str0m 0.24.1** `src/bwe/` and `src/pacer/` (≈9.9 k lines; only internal deps are `Bitrate`, `DataSize`, `TwccSendRecord`, `TwccSeq`, `MidRid`), with str0m's copyright notice kept | MIT (of MIT OR Apache-2.0) | new `crates/rekindle-media-bwe`, `#![forbid(unsafe_code)]` |
| `BitrateAllocator` (one estimate split by priority, min and max across audio 24–64 kbps, video, FEC and screen) and audio accounting at the head of the pacer (`SetAccountForAudioPackets`, r6 I-BW2) | Ported from libwebrtc `call/bitrate_allocator.cc` semantics (r6 §2.1) | BSD-3-Clause | `rekindle-media-bwe` |
| Transport feedback | RFC 8888-shaped report (begin_seq plus per-packet arrival offsets; send times kept by the sender, per evidence `steps-20-26.md` §25 item 2), as a Cap'n Proto struct (CLAUDE.md wire rule), every 50–200 ms on the same route | — | `schemas/voice.capnp`, `rekindle-codec` |
| NetEQ audio playout | **Faithful port of libwebrtc NetEQ**: DelayManager, UnderrunOptimizer, ReorderOptimizer, DecisionLogic, BufferLevelFilter, PacketBuffer, Expand, Merge, Normal, Accelerate, PreemptiveExpand, BackgroundNoise, ComfortNoise and StatisticsCalculator. Opus decode through the existing `opus` 0.3 crate. Its unit tests are ported with it. | BSD-3-Clause | new `crates/rekindle-neteq`, `#![forbid(unsafe_code)]` |
| Video receive timing, keyframe-request policy, A/V sync | Ported from libwebrtc `video/timing/` (jitter estimator and frame-delay Kalman), `video_receive_stream2` constants (200 ms / 3 s) and `video/stream_synchronization.cc` (1 s cadence, 80 ms step) | BSD-3-Clause | `rekindle-video` (stays `forbid(unsafe_code)`) |
| Loss-protection policy (RS FEC weight versus RTT; no audio NACK) | libwebrtc `media_opt_util` hybrid rule (r6 §2.4), driving the existing Reed-Solomon FEC | BSD-3-Clause | `rekindle-video` |

**What this replaces (deleted when the matching E4.3 sub-step lands):**

- `crates/rekindle-voice/src/jitter/` (`mod.rs`, `accelerate.rs`, `tests.rs`) → `rekindle-neteq`.
- `crates/rekindle-video/src/budget.rs`, the voice RR ladder in
  `crates/rekindle-voice/src/send_loop/quality.rs`, and `AppState.{voice_route_pressure_ms,
  video_bitrate_targets}` → estimator plus allocator. The plan already lists these deletions; this
  ADR names their replacement.
- `crates/rekindle-video/src/pacer.rs` and `send_pacer.rs` → one vendored pacer per peer route,
  carrying audio, video and FEC.
- The rate-decision role of `crates/rekindle-voice/src/receiver_report.rs` moves to transport
  feedback. The RFC 3550 report stays for stats and RTT (`rekindle-media-stats` keeps the E-model
  score).
- `src/components/voice/video_call/playout_buffer.ts` and the `LADDER_*` bitrate ladder in
  `video_sender.ts`: the backend owns timing and target rate, and the frontend reports facts (plan
  WS5.8).
- `crates/rekindle-voice/src/mcu_loop.rs` and `mixer.rs` are deleted by E4.4 anyway (B5). This ADR
  does not change that.

**What is kept:**

- SFrame (`media_crypto.rs`, `replay_window.rs`, `rekindle_secrets::sframe`) and packet signatures.
- The route context in `frame_sender.rs`.
- Opus (`codec.rs`), APM (`audio_processing.rs`: the `aec3` port plus `nnnoiseless`) and cpal capture.
- Video fragmentation, reassembly and Reed-Solomon (`fragment.rs`, `reassembler/`).
- The codec layer: WebCodecs, `rekindle-video-libvpx` and `rekindle-video-capture`.
- `rekindle-media-stats`.

### Why Option F over A–E (the reasons)

1. **It is the only option that meets every driver.** It keeps libwebrtc's algorithms and constants
   (driver 1) in memory-safe Rust inside `forbid(unsafe_code)` crates (driver 3). There is no
   DTLS/ICE/SRTP layer (driver 2), no internal C++ API (driver 4) and no binary blobs. It builds
   under the existing Nix shell with no new system libraries.
2. **Discord's precedent transfers in shape, not in substance.** Discord runs a C++ media team and a
   C++ SFU on its own servers (`discord-2.5m-voice.txt:182`). Rekindle's engine runs in a Rust daemon
   of a privacy tool, on peer-supplied packets. The part that does transfer (no SDP, no ICE, own
   transport, own encryption) is exactly what this option does.
3. **str0m's engine is the right code but not the right container.** Its GoogCC is the most complete
   Rust port of libwebrtc's production controller, with probing and ALR, which R-BW5 needs and
   rtc-interceptor lacks. But its session will not move media without DTLS keys, and its BWE is
   private. Vendoring the two modules takes the engine without the session.
4. **NetEQ is the one engine with no reusable Rust implementation.** The only crate is a partial
   "inspired" one. A test-backed port is the only way to meet R-AU1–R-AU4 without libwebrtc. The
   team has already started porting it piece by piece (C7.23 `accelerate.rs`). This decision makes
   that a whole port instead of fragments.

## Consequences

**Positive.**

- One estimator, allocator and pacer per route, sourced and testable. The E4.3 verify gates (netem
  2 Mbps→300 kbps step, standing-queue test, 180 ms p95 jitter replay, clapper) measure ported
  behaviour, not new inventions.
- No new C/C++ in the daemon. The CISA §1 claim stays true. All new code is MIT or BSD-3, with no
  AGPL.
- The engine is transport-agnostic (Sans-I/O style), so it can run in the daemon (ADR 0010) and in
  the E4.4 harness with a virtual clock.

**Negative.**

- Rekindle owns about 10 k vendored lines (str0m BWE/pacer) and about 11 k lines of NetEQ port.
  Upstream fixes must be tracked by hand: str0m's CHANGELOG, and libwebrtc's `neteq/` and
  `video/timing/` history. Each crate records its upstream revision (str0m `393a3d23`, libwebrtc
  `fc2f362a`) in its header.
- BSD-3 and MIT notices must ship in release artefacts. The release packaging (F3) gains a
  third-party-notices file.
- **Plan delta.** E4.3's congestion-control line says "GCC per draft-ietf-rmcat-gcc-02 §5 (Kalman
  filter)". This decision uses libwebrtc's trendline estimator as ported by str0m.
  `evidence/steps-20-26.md` §25 item 1 explicitly allows either choice if the matching source is
  cited. The E4.3 text would be amended to cite `goog_cc/trendline_estimator.cc` and str0m
  `src/bwe/delay/trendline.rs`.
- A Veilid route change is a new path. The estimator resets, as libwebrtc does on a network-route
  change [lead: libwebrtc `OnNetworkRouteChanged` not re-read in this spike], and the UI shows the
  re-probe.
- No engine can fix route latency. If E4.3.0 part 1 measures route p95 one-way delay above about
  250–300 ms, voice is outside G.114 whatever the engine (r6 §7.2). The engine shows this; it cannot
  cure it.

## Verification

- **Port conformance:**
  - `rekindle-neteq` passes Rust ports of libwebrtc's `delay_manager`, `underrun_optimizer`,
    `reorder_optimizer`, `decision_logic`, `buffer_level_filter`, `packet_buffer`, `expand`,
    `merge`, `time_stretch` and `statistics_calculator` unit tests, with the same expected values;
  - the vendored BWE keeps str0m's in-module tests green;
  - the video timing port passes `jitter_estimator` and `frame_delay_variation_kalman_filter` test
    vectors.
- **Boundary checks:**
  - an architecture test finds exactly one estimator type and one pacer per route (r6 R-BW1);
  - grep finds no `budget.rs`, no `jitter/` and no `LADDER_` constants;
  - `cargo tree -i veilid-core` stays empty for the new crates (ADR 0014).
- **Behaviour:** the E4.3 verify list unchanged (r6 R-BW1–R-BW5, R-AU1–R-AU4, R-VI1, R-VI2), run in
  the E4.4 impaired-network harness on Mac ↔ Pop!_OS, plus a Windows build in CI.
- **Licence:** `cargo deny check licenses` passes with str0m (MIT), and the notices file lists
  libwebrtc BSD-3 and str0m MIT.

## More information

- Spike evidence and the source list: `.claude/plans/standards-remediation/evidence/e4-3-0-q5-spike.md`.
- Open questions for the owner are listed at the end of that file.
