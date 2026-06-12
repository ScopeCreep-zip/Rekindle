// Architecture §10.6 — send-side of the interim video pipeline. Owns the
// WebCodecs VideoEncoder, the canvas-capture pump loop, and the per-track
// adaptive-bitrate state. Split out of VideoCallPanel so the orchestration
// hook stays focused on UI lifecycle + the receiver path.
//
// Encoder lifecycle (Phase 2): the negotiated codec is cross-checked
// against the local probe before EVERY configure (an unencodable codec
// must never reach `configure()` — on WKWebView that throw used to kill
// the camera permanently); a failed/closed VideoEncoder is RECREATED
// (a closed encoder is unusable forever per WebCodecs); a first-chunk
// watchdog catches the documented "isConfigSupported says yes but the
// encoder silently produces nothing" platform bug; capture is gated on
// the <video> element actually having pixels (WKWebView black-frame
// hazard); and scheduling prefers requestVideoFrameCallback, which —
// unlike requestAnimationFrame — keeps firing while the window is
// occluded on macOS.
import { commands } from "../../../ipc/commands";
import type { Codec, MediaCapabilities, SessionVideoConfig } from "../../../ipc/commands";
import { localVideoCapabilities } from "../../../handlers/video.handlers";
import {
  dmPeerDecodeCodecsFor,
  videoSessionConfigFor,
} from "../../../stores/video.store";
import {
  KEYFRAME_INTERVAL_MS,
  KEYFRAME_MIN_INTERVAL_MS,
  LADDER_OVERSHOOT_RATIO,
  LADDER_UNDERSHOOT_RATIO,
  LADDER_UP_STREAK,
  LADDER_WINDOW_MS,
  bytesToBase64,
  randomStreamIdHex,
  wireCodecToWebCodecsString,
} from "./codec_utils";

/** Fps/keyframe-cadence steps below the negotiated ceiling. Resolution
 *  NEVER changes mid-stream: a ladder move that reconfigured the
 *  encoder to new dimensions broke both receiving platforms' WebCodecs
 *  decoders on the in-band resolution switch (WebKitGTK stalled with
 *  no output and no error callback; WKWebView painted a black tile).
 *  Fps is floored near 7: under CBR, per-frame bytes = bitrate ÷ fps,
 *  so cutting fps below that point GROWS each frame instead of
 *  shedding bytes — the 2 fps depths of the previous ladder produced
 *  40 KB deltas / 160 KB keyframes (temporal prediction collapses at
 *  500 ms frame spacing) and froze the far end. Bytes are shed by the
 *  fps-coupled encoder bitrate (`effectiveBitrate`) following the AIMD
 *  target down, not by fps alone. Late joiners aren't stranded by the
 *  6 s cadence: the keyframe-request path (proven live) forces one on
 *  demand. */
const LADDER: ReadonlyArray<{ fpsScale: number; kfIntervalMs: number }> = [
  { fpsScale: 1, kfIntervalMs: KEYFRAME_INTERVAL_MS },
  { fpsScale: 0.8, kfIntervalMs: KEYFRAME_INTERVAL_MS },
  { fpsScale: 0.66, kfIntervalMs: 6000 },
  { fpsScale: 0.5, kfIntervalMs: 6000 },
];

export type TrackLabel = "camera" | "screen";

/** W11.4 — `community` routes encoded frames through gossip fan-out + MEK;
 *  `dm` routes 1:1 via Signal Double Ratchet. */
export type SenderRoute =
  | { mode: "community"; communityId: string; channelId: string }
  | { mode: "dm"; peerId: string };

/** How long the encoder may consume frames without producing a single
 *  chunk before the watchdog recreates it. */
const FIRST_CHUNK_DEADLINE_MS = 2_000;
/** Minimum spacing between automatic encoder recreations; a second
 *  failure inside the window is fatal (surfaced, camera stops). */
const RECREATE_COOLDOWN_MS = 10_000;

/** Pure encodability gate: the encoder must never be configured with a
 *  codec the local engine can't encode. `null` caps = probe hasn't
 *  reported — equally not configurable. */
export function pickEncoderCodec(
  local: MediaCapabilities | null,
  negotiated: Codec,
): { ok: boolean; reason?: string } {
  if (!local) {
    return { ok: false, reason: "local capabilities not probed yet" };
  }
  if (!local.encodeCodecs.includes(negotiated)) {
    return {
      ok: false,
      reason: `negotiated codec ${negotiated} not locally encodable (have: ${
        local.encodeCodecs.join(",") || "none"
      })`,
    };
  }
  return { ok: true };
}

/** WebKitGTK 2.52 may lack rVFC — feature-detected, rAF otherwise. */
type VideoWithRVFC = HTMLVideoElement & {
  requestVideoFrameCallback?: (cb: () => void) => number;
  cancelVideoFrameCallback?: (handle: number) => void;
};

interface TrackState {
  encoder: VideoEncoder | null;
  streamId: string | null;
  frameSeq: number;
  lastKeyframeMs: number;
  // Architecture §10.6 line 4081 — minimum reported downstream kbps across
  // all current receivers; the next configure() caps output to this so the
  // slowest peer keeps pace.
  lowestReceiverKbps: number;
  stop: (() => void) | null;
}

function freshTrack(): TrackState {
  // 350 kbps start — what multi-hop Veilid routes realistically sustain
  // for 480p15 alongside the reserved voice budget (Phase 4 budget.rs).
  return { encoder: null, streamId: null, frameSeq: 0, lastKeyframeMs: 0, lowestReceiverKbps: 350, stop: null };
}

export interface VideoSender {
  start(label: TrackLabel, stream: MediaStream): Promise<void>;
  stop(label: TrackLabel): void;
  /** Force the next encoded frame on the matching stream to be a keyframe. */
  forceKeyframe(streamId: string): void;
  /** Force a keyframe on EVERY active local stream — RFC 5104 FIR
   *  semantics for "a new member entered the conference". */
  forceKeyframeAll(): void;
  /** Follow the backend bitrate policy's target (Phase 4 — assignment,
   *  not a min-clamp: the backend already ran the AIMD + audio-reserve
   *  math over receiver feedback). Applies to both tracks. */
  setTargetKbps(kbps: number): void;
}

export function createVideoSender(
  route: SenderRoute,
  onError: (msg: string) => void,
): VideoSender {
  const tracks: Record<TrackLabel, TrackState> = {
    camera: freshTrack(),
    screen: freshTrack(),
  };

  /** Backend log sink for encoder lifecycle events — the WKWebView /
   *  WebKitGTK divergence must be visible in `RUST_LOG` traces, not
   *  only in a devtools console nobody has open. */
  function reportEncoderStatus(codec: Codec, ok: boolean, detail: string): void {
    void commands
      .reportVideoEncoderStatus(
        route.mode === "community" ? route.communityId : null,
        route.mode === "dm" ? route.peerId : null,
        codec,
        ok,
        detail,
      )
      .catch(() => {
        // Logging side-channel only — never disturb the pipeline.
      });
  }

  /** Phase 5 — first of OUR probed encode codecs the DM peer can
   *  decode (their list rides CallInvite/CallAccept). An empty peer
   *  list means "unknown" (pre-fetch race / pre-probe peer): the vp9
   *  wire floor applies, but ONLY when we can actually encode it —
   *  otherwise wait (`null`); the store write from the caps fetch
   *  re-runs this via the pump. */
  function pickDmEncoderCodec(peerId: string): Codec | null {
    const local = localVideoCapabilities();
    if (!local) return null;
    const peerDecode = dmPeerDecodeCodecsFor(peerId);
    if (!peerDecode || peerDecode.length === 0) {
      return local.encodeCodecs.includes("vp9") ? "vp9" : null;
    }
    return local.encodeCodecs.find((c) => peerDecode.includes(c)) ?? null;
  }

  /** The negotiated encoder constraints, or `null` when no encodable
   *  shape exists YET (community: config not in store — only possible
   *  mid-call during renegotiation, the media-ready gate guarantees a
   *  config before camera start; DM: caps/peer-list race). The pump
   *  idles on `null` and picks up the store write on a later tick. */
  function encoderConstraints(): SessionVideoConfig["encoder"] | null {
    if (route.mode === "community") {
      return videoSessionConfigFor(route.communityId, route.channelId)?.encoder ?? null;
    }
    const codec = pickDmEncoderCodec(route.peerId);
    if (codec === null) return null;
    return {
      codec,
      maxWidth: 854,
      maxHeight: 480,
      maxFps: 15,
      scalabilityMode: "flat",
    };
  }

  /** Build the WebCodecs config for `encoder.configure()`. `bitrate` is
   *  rebound per-call because the adaptive loop varies it independently
   *  of the negotiated capability shape. H.264 encodes Annex-B so
   *  SPS/PPS ride the bitstream — receivers configure their decoder
   *  codec-string-only, no avcC `description` plumbing. */
  function buildEncoderConfig(
    constraints: SessionVideoConfig["encoder"],
    bitrate: number,
    shape?: { width: number; height: number; fps: number },
  ): VideoEncoderConfig {
    const base: VideoEncoderConfig = {
      codec: wireCodecToWebCodecsString(constraints.codec),
      width: shape?.width ?? constraints.maxWidth,
      height: shape?.height ?? constraints.maxHeight,
      framerate: shape?.fps ?? constraints.maxFps,
      bitrate,
      latencyMode: "realtime",
      ...(constraints.codec === "h264" ? { avc: { format: "annexb" as const } } : {}),
    };
    return constraints.scalabilityMode === "l1t2"
      ? { ...base, scalabilityMode: "L1T2" }
      : base;
  }

  async function start(label: TrackLabel, stream: MediaStream): Promise<void> {
    const ts = tracks[label];
    // Community streams use the deterministic backend-derived id so
    // (channel_id || sender_pseudonym || track_label) collisions are
    // impossible across concurrent senders. DM streams are 1:1 — a local
    // random 16-byte UUID is sufficient and avoids a backend round-trip.
    const streamIdHex =
      route.mode === "community"
        ? await commands.deriveVideoStreamId(route.communityId, route.channelId, label)
        : randomStreamIdHex();
    const track = stream.getVideoTracks()[0];
    if (!track) throw new Error("no video track");

    // Drive the canvas off a hidden <video> element so the same
    // MediaStream backs both the local preview tile and the encoder
    // input. WKWebView / WebKitGTK don't implement
    // MediaStreamTrackProcessor, so we draw frames to an offscreen
    // canvas and build VideoFrames from it instead.
    const captureVideo = document.createElement("video");
    captureVideo.srcObject = stream;
    captureVideo.muted = true;
    captureVideo.playsInline = true;
    await captureVideo.play().catch((e) => {
      console.error("capture <video> play failed:", e);
    });

    let constraints = encoderConstraints();
    if (constraints === null) {
      // Community mode is media-ready-gated, so this only fires on a
      // DM pre-caps race or a config torn down mid-toggle. Hard error
      // — the toggle reverts and the user retries once connected.
      throw new Error("no negotiated encoder config yet — video session still connecting");
    }
    {
      const check = pickEncoderCodec(localVideoCapabilities(), constraints.codec);
      if (!check.ok) {
        reportEncoderStatus(constraints.codec, false, `start: ${check.reason}`);
        throw new Error(`cannot start video: ${check.reason}`);
      }
    }

    // The codec the encoder is CURRENTLY configured for — every encoded
    // chunk is tagged with it (RTP payload-type analog). Updated only
    // after a codec-changing reconfigure (post-flush) so queued chunks
    // of the old codec keep their truthful tag.
    let currentCodec = constraints.codec;

    // Output-measured ladder state (see codec_utils LADDER_* rationale):
    // level indexes LADDER; bytes/windowStart accumulate real encoder
    // output between evaluations. Width/height stay at the negotiated
    // constraints for the stream's whole life (see LADDER comment).
    let ladderLevel = 0;
    let ladderBytes = 0;
    let ladderWindowStart = performance.now();
    let ladderUpStreak = 0;
    const appliedShape = (): { width: number; height: number; fps: number } => {
      const step = LADDER[ladderLevel];
      return {
        width: constraints!.maxWidth,
        height: constraints!.maxHeight,
        fps: Math.max(1, Math.round(constraints!.maxFps * step.fpsScale)),
      };
    };
    const ladderKfIntervalMs = (): number => LADDER[ladderLevel].kfIntervalMs;

    /** Encoder bitrate coupled to the EFFECTIVE fps. Under CBR,
     *  per-frame bytes = bitrate ÷ fps — handing the full AIMD target
     *  to a low-fps stream concentrates the whole budget into a few
     *  giant frames (live: 1200 kbps at 2 fps = 75 KB average frames,
     *  ~190 KB keyframes via libvpx's 250% max-intra, each costing
     *  seconds of pacer drain — the frozen-tile chain). Scaling by
     *  fps/5 bounds a keyframe to ~½ s of pacer budget at any level;
     *  at ≥5 fps the full target applies. Floor keeps the encoder out
     *  of its degenerate sub-50 kbps range. */
    const effectiveBitrate = (kbps: number, fps: number): number =>
      Math.max(50_000, Math.round(kbps * 1000 * Math.min(1, fps / 5)));

    const captureCanvas = document.createElement("canvas");
    captureCanvas.width = constraints.maxWidth;
    captureCanvas.height = constraints.maxHeight;
    const captureCtx = captureCanvas.getContext("2d");
    if (!captureCtx) throw new Error("2d context unavailable");

    let cancelled = false;
    let fatal = false;
    let configuredKbps = ts.lowestReceiverKbps;
    // Watchdog state: frames fed vs chunks produced since the last
    // (re)configure. A healthy encoder produces its first chunk within
    // one frame interval; FIRST_CHUNK_DEADLINE_MS of silence means the
    // platform encoder is broken despite isConfigSupported's promise.
    let framesFed = 0;
    let chunksOut = 0;
    let firstFedAt = 0;
    let lastRecreateAt = 0;
    // Report an unencodable mid-call renegotiation only once per codec.
    let reportedUnencodable: Codec | null = null;

    const makeEncoder = (): VideoEncoder =>
      new VideoEncoder({
        output: (chunk: EncodedVideoChunk) => {
          chunksOut += 1;
          ladderBytes += chunk.byteLength;
          const buf = new Uint8Array(chunk.byteLength);
          chunk.copyTo(buf);
          const payloadB64 = bytesToBase64(buf);
          const seq = (ts.frameSeq += 1);
          const frameRequest = {
            streamIdHex,
            frameSeq: seq,
            keyframe: chunk.type === "key",
            codec: currentCodec,
            timestamp: Math.floor(performance.now()),
            encodedPayloadB64: payloadB64,
          };
          if (route.mode === "community") {
            void commands.sendVideoFrame(route.communityId, route.channelId, frameRequest);
          } else {
            void commands.sendDmVideoFrame(route.peerId, frameRequest);
          }
        },
        error: (e: Error) => {
          // A WebCodecs error callback leaves the encoder closed and
          // permanently unusable — recreate, don't limp.
          console.error("VideoEncoder error:", e);
          recreateEncoder(`error-callback: ${e.message}`);
        },
      });

    let encoder = makeEncoder();

    /** Tear down + rebuild the encoder after a platform failure. One
     *  automatic recovery per cooldown window; a second failure inside
     *  it is fatal (camera stops, error surfaced + reported). */
    const recreateEncoder = (reason: string): void => {
      if (cancelled || fatal || constraints === null) return;
      const now = performance.now();
      if (lastRecreateAt !== 0 && now - lastRecreateAt < RECREATE_COOLDOWN_MS) {
        fatal = true;
        reportEncoderStatus(currentCodec, false, `fatal after recreate: ${reason}`);
        onError("Video encoder repeatedly failing — camera stopped");
        return;
      }
      lastRecreateAt = now;
      try {
        encoder.close();
      } catch {
        // Already closed — that's why we're here.
      }
      encoder = makeEncoder();
      try {
        encoder.configure(buildEncoderConfig(constraints, effectiveBitrate(configuredKbps, appliedShape().fps), appliedShape()));
        currentCodec = constraints.codec;
        ts.lastKeyframeMs = 0; // force a keyframe so receivers re-sync
        framesFed = 0;
        chunksOut = 0;
        firstFedAt = 0;
        // Stale output bytes from the dead encoder must not skew the
        // next ladder evaluation.
        ladderBytes = 0;
        ladderWindowStart = now;
        console.warn(`video encoder recreated (${reason})`);
        reportEncoderStatus(currentCodec, true, `recreated: ${reason}`);
      } catch (e) {
        fatal = true;
        const msg = e instanceof Error ? e.message : String(e);
        reportEncoderStatus(constraints.codec, false, `reconfigure after recreate: ${msg}`);
        onError(`Encoder unrecoverable: ${msg}`);
      }
    };

    // Initial configure — the cross-check above guarantees the codec is
    // locally encodable, but WebKit can still reject the full config
    // shape; surface that instead of letting startCamera die opaquely.
    try {
      encoder.configure(buildEncoderConfig(constraints, effectiveBitrate(configuredKbps, appliedShape().fps), appliedShape()));
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      reportEncoderStatus(constraints.codec, false, `initial configure: ${msg}`);
      onError(`Encoder configure failed: ${msg}`);
      throw e instanceof Error ? e : new Error(msg);
    }
    reportEncoderStatus(currentCodec, true, "configured");

    let frameIntervalMs = 1000 / appliedShape().fps;
    let lastEmittedAt = 0;
    let scheduleHandle: number | null = null;
    let scheduledVia: "rvfc" | "raf" | "timer" = "raf";

    const cancelScheduled = (): void => {
      if (scheduleHandle === null) return;
      const v = captureVideo as VideoWithRVFC;
      if (scheduledVia === "rvfc" && typeof v.cancelVideoFrameCallback === "function") {
        v.cancelVideoFrameCallback(scheduleHandle);
      } else if (scheduledVia === "raf") {
        cancelAnimationFrame(scheduleHandle);
      } else {
        clearTimeout(scheduleHandle);
      }
      scheduleHandle = null;
    };

    /** Prefer requestVideoFrameCallback (fires per camera frame, keeps
     *  running while the window is occluded on macOS — rAF doesn't);
     *  rAF when the engine lacks rVFC (WebKitGTK 2.52). Hidden page:
     *  timer chain — every platform's webview suspends rAF for hidden
     *  or minimized windows (Page Visibility semantics), so an
     *  rAF-scheduled pump freezes and outbound video goes static while
     *  keyframe requests pile up unanswered. Hidden-page timers are
     *  clamped (~1 s), but 1 fps with serviced keyframe requests beats
     *  a frozen tile. */
    const scheduleNext = (): void => {
      if (cancelled || fatal) return;
      const v = captureVideo as VideoWithRVFC;
      if (document.hidden) {
        scheduledVia = "timer";
        scheduleHandle = window.setTimeout(pump, Math.max(frameIntervalMs, 250));
      } else if (typeof v.requestVideoFrameCallback === "function") {
        scheduledVia = "rvfc";
        scheduleHandle = v.requestVideoFrameCallback(pump);
      } else {
        scheduledVia = "raf";
        scheduleHandle = requestAnimationFrame(pump);
      }
    };

    /** Re-arm across the visibility edge: a pending rAF/rVFC in a
     *  newly-hidden window may simply never fire — the pump chain dies
     *  before it can reschedule itself onto the timer path. */
    const onVisibilityChange = (): void => {
      if (cancelled || fatal) return;
      cancelScheduled();
      scheduleNext();
    };
    document.addEventListener("visibilitychange", onVisibilityChange);

    const pump = (): void => {
      if (cancelled || fatal) return;
      // Phase C — pick up any negotiated-config change between frames.
      // `null` = renegotiation in flight (config torn down) — idle
      // without touching the running encoder; the next emit restores it.
      const fresh = encoderConstraints();
      if (fresh === null) {
        scheduleNext();
        return;
      }
      const constraintsChanged =
        fresh.codec !== constraints!.codec ||
        fresh.maxWidth !== constraints!.maxWidth ||
        fresh.maxHeight !== constraints!.maxHeight ||
        fresh.maxFps !== constraints!.maxFps ||
        fresh.scalabilityMode !== constraints!.scalabilityMode;
      if (constraintsChanged) {
        // Never reconfigure INTO a codec this platform can't encode —
        // keep the current (working) encoder and report once.
        const check = pickEncoderCodec(localVideoCapabilities(), fresh.codec);
        if (!check.ok) {
          if (reportedUnencodable !== fresh.codec) {
            reportedUnencodable = fresh.codec;
            console.warn(`ignoring renegotiated config: ${check.reason}`);
            reportEncoderStatus(fresh.codec, false, `renegotiation: ${check.reason}`);
          }
          scheduleNext();
          return;
        }
        reportedUnencodable = null;
        const codecChanged = fresh.codec !== currentCodec;
        constraints = fresh;
        // New negotiated ceiling — re-discover the sustainable shape
        // below it from scratch rather than carry over a ladder level
        // measured against the old one.
        ladderLevel = 0;
        ladderBytes = 0;
        ladderWindowStart = performance.now();
        ladderUpStreak = 0;
        const shape = appliedShape();
        captureCanvas.width = shape.width;
        captureCanvas.height = shape.height;
        frameIntervalMs = 1000 / shape.fps;
        const reconfigure = (): void => {
          try {
            encoder.configure(buildEncoderConfig(constraints!, effectiveBitrate(configuredKbps, appliedShape().fps), appliedShape()));
            currentCodec = constraints!.codec;
            ts.lastKeyframeMs = performance.now();
            framesFed = 0;
            chunksOut = 0;
            firstFedAt = 0;
          } catch (e) {
            console.error("encoder reconfigure on policy change failed:", e);
            recreateEncoder("policy-reconfigure-failed");
          }
        };
        if (codecChanged) {
          // Drain chunks queued under the old codec so their tag stays
          // truthful, THEN reconfigure. A missed flush self-heals via
          // the receiver's decode-error → KeyframeRequest path.
          void encoder
            .flush()
            .catch((e: unknown) => {
              console.error("encoder flush before codec switch failed:", e);
            })
            .finally(reconfigure);
        } else {
          reconfigure();
        }
      }
      // A closed encoder silently eats every encode() — recover.
      if (encoder.state === "closed") {
        recreateEncoder("closed-state");
        scheduleNext();
        return;
      }
      const now = performance.now();
      if (now - lastEmittedAt >= frameIntervalMs) {
        // Architecture §10.6 line 4081 — adapt to the slowest receiver's
        // measured kbps (from real frame acks). reconfigure() forces a
        // keyframe, so only act on a material (>15%) drift to avoid churn.
        if (Math.abs(ts.lowestReceiverKbps - configuredKbps) / configuredKbps > 0.15) {
          configuredKbps = ts.lowestReceiverKbps;
          try {
            encoder.configure(buildEncoderConfig(constraints!, effectiveBitrate(configuredKbps, appliedShape().fps), appliedShape()));
            ts.lastKeyframeMs = now; // reconfigure already emits a keyframe
          } catch (e) {
            console.error("encoder reconfigure failed:", e);
            recreateEncoder("bitrate-reconfigure-failed");
            scheduleNext();
            return;
          }
        }
        // WKWebView hazard: drawing a not-yet-decodable <video> yields
        // black pixels (or throws). Skip the tick until the element
        // actually has current frame data.
        if (captureVideo.readyState < 2 || captureVideo.videoWidth === 0) {
          scheduleNext();
          return;
        }
        try {
          captureCtx.drawImage(captureVideo, 0, 0, captureCanvas.width, captureCanvas.height);
          const isKeyframe = now - ts.lastKeyframeMs >= ladderKfIntervalMs();
          if (isKeyframe) ts.lastKeyframeMs = now;
          // Explicit duration: canvas-sourced frames otherwise carry
          // WebKitGTK's hardcoded 1-second GstBuffer duration, which
          // whipsaws libvpx's framerate belief every frame and wrecks
          // its rate control (one leg of the observed CBR overshoot).
          const videoFrame = new VideoFrame(captureCanvas, {
            timestamp: Math.floor(now * 1000),
            duration: Math.round(frameIntervalMs * 1000),
          });
          encoder.encode(videoFrame, { keyFrame: isKeyframe });
          videoFrame.close();
          framesFed += 1;
          if (firstFedAt === 0) firstFedAt = now;
        } catch (e) {
          console.error("encode failed:", e);
        }
        lastEmittedAt = now;
      }
      // Output-measured ladder: compare REAL encoder output against the
      // bitrate target and step fps/keyframe-cadence until it fits —
      // the configured bitrate is loosely honored on WebKitGTK (VP9
      // rides libvpx GOOD-quality deadline, not realtime). Windows with
      // zero output (encoder warming/stalled) are skipped: silence is
      // not headroom.
      if (now - ladderWindowStart >= LADDER_WINDOW_MS) {
        const measuredKbps = (ladderBytes * 8) / (now - ladderWindowStart);
        const produced = ladderBytes > 0;
        ladderBytes = 0;
        ladderWindowStart = now;
        if (produced) {
          let next = ladderLevel;
          if (
            measuredKbps > configuredKbps * LADDER_OVERSHOOT_RATIO &&
            ladderLevel < LADDER.length - 1
          ) {
            next = ladderLevel + 1;
            ladderUpStreak = 0;
          } else if (
            measuredKbps < configuredKbps * LADDER_UNDERSHOOT_RATIO &&
            ladderLevel > 0
          ) {
            ladderUpStreak += 1;
            if (ladderUpStreak >= LADDER_UP_STREAK) {
              next = ladderLevel - 1;
              ladderUpStreak = 0;
            }
          } else {
            ladderUpStreak = 0;
          }
          if (next !== ladderLevel) {
            ladderLevel = next;
            // Same-dimension reconfigure (the proven-safe class — the
            // drift path does it): the encoder must hear the TRUTHFUL
            // framerate and the fps-coupled bitrate, or CBR keeps
            // splitting the old budget across the new frame count and
            // each frame balloons (the 2 fps / 75 KB-frame failure).
            // Resolution never changes here.
            const shape = appliedShape();
            frameIntervalMs = 1000 / shape.fps;
            try {
              encoder.configure(
                buildEncoderConfig(constraints!, effectiveBitrate(configuredKbps, shape.fps), shape),
              );
              ts.lastKeyframeMs = now; // reconfigure already emits a keyframe
            } catch (e) {
              console.error("ladder reconfigure failed:", e);
              recreateEncoder("ladder-reconfigure-failed");
              scheduleNext();
              return;
            }
            reportEncoderStatus(
              currentCodec,
              true,
              `ladder ${ladderLevel}: ${shape.fps}fps kf=${ladderKfIntervalMs()}ms ` +
                `bitrate=${Math.round(effectiveBitrate(configuredKbps, shape.fps) / 1000)}kbps ` +
                `measured=${Math.round(measuredKbps)}kbps target=${configuredKbps}kbps`,
            );
          }
        }
      }
      // First-chunk watchdog: frames going in, nothing coming out.
      if (
        chunksOut === 0 &&
        framesFed > 0 &&
        firstFedAt !== 0 &&
        now - firstFedAt > FIRST_CHUNK_DEADLINE_MS
      ) {
        // The recreate cooldown makes a second dry window fatal.
        recreateEncoder("no-output-watchdog");
      }
      scheduleNext();
    };
    scheduleNext();

    ts.encoder = encoder;
    ts.streamId = streamIdHex;

    // Architecture §10.6 + Phase 6 W22 — community broadcasts initial
    // topology so receivers spin up decoders. DM has only one receiver who
    // spins up their decoder on the first keyframe (in ingestRemoteFrame),
    // so no topology broadcast is needed.
    if (route.mode === "community") {
      void commands.notifyVideoTopologyChange(
        route.communityId,
        route.channelId,
        streamIdHex,
        null,
        "initial",
      );
    }

    ts.stop = () => {
      cancelled = true;
      document.removeEventListener("visibilitychange", onVisibilityChange);
      cancelScheduled();
      try {
        encoder.close();
      } catch (e) {
        console.error("encoder close failed:", e);
      }
      captureVideo.srcObject = null;
    };
  }

  function stop(label: TrackLabel): void {
    const ts = tracks[label];
    ts.stop?.();
    ts.stop = null;
    ts.encoder = null;
    ts.streamId = null;
  }

  /** Honor a force only when the last keyframe is older than the
   *  libwebrtc-style send floor — receivers re-request at 1 Hz until
   *  resynced, so an unthrottled force would emit a keyframe per
   *  request and starve the pacer with 30-100 KB intras.
   *  `lastKeyframeMs = 0` is the "emit on next tick" sentinel. */
  const forceIfDue = (ts: TrackState): void => {
    if (ts.lastKeyframeMs !== 0 && performance.now() - ts.lastKeyframeMs < KEYFRAME_MIN_INTERVAL_MS) {
      return;
    }
    ts.lastKeyframeMs = 0;
  };

  function forceKeyframe(streamId: string): void {
    for (const ts of Object.values(tracks)) {
      if (ts.streamId === streamId) forceIfDue(ts);
    }
  }

  function forceKeyframeAll(): void {
    for (const ts of Object.values(tracks)) {
      if (ts.streamId !== null) forceIfDue(ts);
    }
  }

  function setTargetKbps(kbps: number): void {
    for (const ts of Object.values(tracks)) {
      ts.lowestReceiverKbps = kbps;
    }
  }

  return { start, stop, forceKeyframe, forceKeyframeAll, setTargetKbps };
}
