// Architecture §10.6 — send-side of the interim video pipeline. Owns the
// WebCodecs VideoEncoder, the canvas-capture pump loop, and the per-track
// adaptive-bitrate state. Split out of VideoCallPanel so the orchestration
// hook stays focused on UI lifecycle + the receiver path.
import { commands } from "../../../ipc/commands";
import {
  ENCODE_WIDTH,
  ENCODE_HEIGHT,
  ENCODE_FPS,
  KEYFRAME_INTERVAL_MS,
  bytesToBase64,
  randomStreamIdHex,
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

    const captureCanvas = document.createElement("canvas");
    captureCanvas.width = ENCODE_WIDTH;
    captureCanvas.height = ENCODE_HEIGHT;
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
    encoder.configure({
      codec: "vp09.00.30.08",
      width: ENCODE_WIDTH,
      height: ENCODE_HEIGHT,
      framerate: ENCODE_FPS,
      bitrate: 800_000,
      latencyMode: "realtime",
    });

    let cancelled = false;
    const frameIntervalMs = 1000 / ENCODE_FPS;
    let lastEmittedAt = 0;
    let rafHandle: number | null = null;

    const pump = (): void => {
      if (cancelled) return;
      const now = performance.now();
      if (now - lastEmittedAt >= frameIntervalMs) {
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

  return { start, stop, forceKeyframe, noteReceiverKbps, noteBandwidth };
}
