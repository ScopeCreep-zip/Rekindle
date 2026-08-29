// Send-side types, constants, and the fps/keyframe ladder policy —
// extracted from video_sender.ts so the encoder pump file stays focused
// on the (heavily stateful) capture/encode loop.

import { KEYFRAME_INTERVAL_MS } from "./codec_utils";

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
export const LADDER: ReadonlyArray<{ fpsScale: number; kfIntervalMs: number }> = [
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
export const FIRST_CHUNK_DEADLINE_MS = 2_000;
/** Minimum spacing between automatic encoder recreations; a second
 *  failure inside the window is fatal (surfaced, camera stops). */
export const RECREATE_COOLDOWN_MS = 10_000;

/** WebKitGTK 2.52 may lack rVFC — feature-detected, rAF otherwise. */
export type VideoWithRVFC = HTMLVideoElement & {
  requestVideoFrameCallback?: (cb: () => void) => number;
  cancelVideoFrameCallback?: (handle: number) => void;
};

export interface TrackState {
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

export function freshTrack(): TrackState {
  // 350 kbps start — what multi-hop Veilid routes realistically sustain
  // for 480p15 alongside the reserved voice budget (Phase 4 budget.rs).
  return {
    encoder: null,
    streamId: null,
    frameSeq: 0,
    lastKeyframeMs: 0,
    lowestReceiverKbps: 350,
    stop: null,
  };
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
