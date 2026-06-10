// Architecture §10.6 — send-side of the interim video pipeline. Owns the
// WebCodecs VideoEncoder, the canvas-capture pump loop, and the per-track
// adaptive-bitrate state. Split out of VideoCallPanel so the orchestration
// hook stays focused on UI lifecycle + the receiver path.
import { commands } from "../../../ipc/commands";
import type { Codec, SessionVideoConfig } from "../../../ipc/commands";
import { localVideoCapabilities } from "../../../handlers/video.handlers";
import {
  dmPeerDecodeCodecsFor,
  videoSessionConfigFor,
} from "../../../stores/video.store";
import {
  KEYFRAME_INTERVAL_MS,
  bytesToBase64,
  randomStreamIdHex,
  wireCodecToWebCodecsString,
} from "./codec_utils";

export type TrackLabel = "camera" | "screen";

/** W11.4 — `community` routes encoded frames through gossip fan-out + MEK;
 *  `dm` routes 1:1 via Signal Double Ratchet. */
export type SenderRoute =
  | { mode: "community"; communityId: string; channelId: string }
  | { mode: "dm"; peerId: string };

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
  return { encoder: null, streamId: null, frameSeq: 0, lastKeyframeMs: 0, lowestReceiverKbps: 800, stop: null };
}

export interface VideoSender {
  start(label: TrackLabel, stream: MediaStream): Promise<void>;
  stop(label: TrackLabel): void;
  /** Force the next encoded frame on the matching stream to be a keyframe. */
  forceKeyframe(streamId: string): void;
  /** Force a keyframe on EVERY active local stream — RFC 5104 FIR
   *  semantics for "a new member entered the conference". */
  forceKeyframeAll(): void;
  /** Clamp a track's reported downstream kbps to the receiver's estimate. */
  noteReceiverKbps(streamId: string, kbps: number): void;
  /** Channel-scoped bandwidth hint — clamps both tracks. */
  noteBandwidth(kbps: number): void;
}

export function createVideoSender(
  route: SenderRoute,
  onError: (msg: string) => void,
): VideoSender {
  const tracks: Record<TrackLabel, TrackState> = {
    camera: freshTrack(),
    screen: freshTrack(),
  };

  /** Phase 5 — first of OUR probed encode codecs the DM peer can
   *  decode (their list rides CallInvite/CallAccept). Falls back to
   *  the VP9 floor while either side of the intersection is unknown —
   *  the pre-caps race is safe because receivers create decoders from
   *  per-fragment tags, and the pump's constraints check re-runs this
   *  every frame, so the pick self-corrects once the fetch lands. */
  function pickDmEncoderCodec(peerId: string): Codec {
    const local = localVideoCapabilities();
    const peerDecode = dmPeerDecodeCodecsFor(peerId);
    if (local && peerDecode && peerDecode.length > 0) {
      const pick = local.encodeCodecs.find((c) => peerDecode.includes(c));
      if (pick) return pick;
    }
    return "vp9";
  }

  /** Phase C — read the negotiated encoder constraints. Community mode
   *  reads the backend-emitted `SessionVideoConfig` (the backend
   *  negotiator already picked our per-node encoder codec); DM mode
   *  (1:1) intersects locally via `pickDmEncoderCodec`. */
  function encoderConstraints(): SessionVideoConfig["encoder"] {
    if (route.mode === "community") {
      const config = videoSessionConfigFor(route.communityId, route.channelId);
      if (config) return config.encoder;
    }
    return {
      codec: route.mode === "dm" ? pickDmEncoderCodec(route.peerId) : "vp9",
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
  ): VideoEncoderConfig {
    const base: VideoEncoderConfig = {
      codec: wireCodecToWebCodecsString(constraints.codec),
      width: constraints.maxWidth,
      height: constraints.maxHeight,
      framerate: constraints.maxFps,
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
    // The codec the encoder is CURRENTLY configured for — every encoded
    // chunk is tagged with it (RTP payload-type analog). Updated only
    // after a codec-changing reconfigure (post-flush) so queued chunks
    // of the old codec keep their truthful tag.
    let currentCodec = constraints.codec;
    const captureCanvas = document.createElement("canvas");
    captureCanvas.width = constraints.maxWidth;
    captureCanvas.height = constraints.maxHeight;
    const captureCtx = captureCanvas.getContext("2d");
    if (!captureCtx) throw new Error("2d context unavailable");

    const encoder = new VideoEncoder({
      output: (chunk: EncodedVideoChunk) => {
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
        console.error("VideoEncoder error:", e);
        onError(`Encoder error: ${e.message}`);
      },
    });
    // Phase C — configure once from the backend-negotiated constraints.
    // The negotiator already intersected every peer's L1T2 support; no
    // unilateral probe / branch. Start at the slowest receiver's
    // estimate so we never over-send the weakest peer.
    encoder.configure(buildEncoderConfig(constraints, ts.lowestReceiverKbps * 1000));
    let configuredKbps = ts.lowestReceiverKbps;

    let cancelled = false;
    let frameIntervalMs = 1000 / constraints.maxFps;
    let lastEmittedAt = 0;
    let rafHandle: number | null = null;

    const pump = (): void => {
      if (cancelled) return;
      // Phase C — pick up any negotiated-config change between frames.
      // The receiver-side hook tears down decoders on the same trigger
      // — both sides reconfigure together so the next keyframe lands on
      // a matching pipeline. Comparing the codec/width/height/fps/mode
      // shape covers every config-driven encoder.configure() input.
      const fresh = encoderConstraints();
      const constraintsChanged =
        fresh.codec !== constraints.codec ||
        fresh.maxWidth !== constraints.maxWidth ||
        fresh.maxHeight !== constraints.maxHeight ||
        fresh.maxFps !== constraints.maxFps ||
        fresh.scalabilityMode !== constraints.scalabilityMode;
      if (constraintsChanged) {
        const codecChanged = fresh.codec !== currentCodec;
        constraints = fresh;
        captureCanvas.width = constraints.maxWidth;
        captureCanvas.height = constraints.maxHeight;
        frameIntervalMs = 1000 / constraints.maxFps;
        const reconfigure = (): void => {
          try {
            encoder.configure(buildEncoderConfig(constraints, configuredKbps * 1000));
            currentCodec = constraints.codec;
            ts.lastKeyframeMs = performance.now();
          } catch (e) {
            console.error("encoder reconfigure on policy change failed:", e);
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
      const now = performance.now();
      if (now - lastEmittedAt >= frameIntervalMs) {
        // Architecture §10.6 line 4081 — adapt to the slowest receiver's
        // measured kbps (from real frame acks). reconfigure() forces a
        // keyframe, so only act on a material (>15%) drift to avoid churn.
        if (Math.abs(ts.lowestReceiverKbps - configuredKbps) / configuredKbps > 0.15) {
          configuredKbps = ts.lowestReceiverKbps;
          try {
            encoder.configure(buildEncoderConfig(constraints, configuredKbps * 1000));
            ts.lastKeyframeMs = now; // reconfigure already emits a keyframe
          } catch (e) {
            console.error("encoder reconfigure failed:", e);
          }
        }
        try {
          captureCtx.drawImage(captureVideo, 0, 0, captureCanvas.width, captureCanvas.height);
          const isKeyframe = now - ts.lastKeyframeMs >= KEYFRAME_INTERVAL_MS;
          if (isKeyframe) ts.lastKeyframeMs = now;
          const videoFrame = new VideoFrame(captureCanvas, {
            timestamp: Math.floor(now * 1000),
          });
          encoder.encode(videoFrame, { keyFrame: isKeyframe });
          videoFrame.close();
        } catch (e) {
          console.error("encode failed:", e);
        }
        lastEmittedAt = now;
      }
      rafHandle = requestAnimationFrame(pump);
    };
    rafHandle = requestAnimationFrame(pump);

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
      if (rafHandle !== null) cancelAnimationFrame(rafHandle);
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

  function forceKeyframe(streamId: string): void {
    for (const ts of Object.values(tracks)) {
      if (ts.streamId === streamId) ts.lastKeyframeMs = 0;
    }
  }

  function forceKeyframeAll(): void {
    for (const ts of Object.values(tracks)) {
      if (ts.streamId !== null) ts.lastKeyframeMs = 0;
    }
  }

  function noteReceiverKbps(streamId: string, kbps: number): void {
    for (const ts of Object.values(tracks)) {
      if (ts.streamId === streamId) {
        ts.lowestReceiverKbps = Math.min(ts.lowestReceiverKbps, kbps);
      }
    }
  }

  function noteBandwidth(kbps: number): void {
    for (const ts of Object.values(tracks)) {
      ts.lowestReceiverKbps = Math.min(ts.lowestReceiverKbps, kbps);
    }
  }

  return { start, stop, forceKeyframe, forceKeyframeAll, noteReceiverKbps, noteBandwidth };
}
