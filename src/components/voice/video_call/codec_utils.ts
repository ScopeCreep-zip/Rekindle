// Architecture §10.6 — shared constants + pure helpers for the interim
// WebCodecs video pipeline. Split out of VideoCallPanel so the sender
// (video_sender.ts) and the orchestration hook can share them without a
// circular import.
//
// Phase A / C — resolution / fps / codec are NOT defined here. The
// backend's `rekindle_video::policy::negotiate_session_config()` emits a
// `SessionVideoConfig` on `CommunityEvent::VideoSessionConfig` and both
// the encoder (`video_sender.ts`) and decoder (`useVideoCallPanel.ts`)
// configure from that store value. The constants below are runtime
// tuning (playout buffer + keyframe cadence + diagnostics), not
// capability negotiation.
import type { VideoPlayoutBuffer } from "./playout_buffer";
import type { Codec } from "../../../ipc/commands/types_sync";

// 4s: at the 350 kbps budget a 30-50 KB keyframe costs ~0.7-1.1 s of
// bucket — a 2 s cadence spent half the budget on keyframes alone.
// Join-time recovery stays fast via FIR-on-confirmed + the receiver's
// 15-dropped-deltas keyframe-request escalation.
export const KEYFRAME_INTERVAL_MS = 4000;

// Keyframe-request (PLI/FIR analog) rate limits, both ends. Sender:
// ignore requests within 300 ms of the last emitted keyframe —
// libwebrtc's kMinKeyframeSendIntervalMs (encoder_rtcp_feedback.cc).
// Receiver: re-request no faster than 1 Hz per stream but PERSIST
// until a keyframe lands — the request envelope is fire-and-forget
// over an onion route, so a lost one-shot means a frozen tile.
export const KEYFRAME_MIN_INTERVAL_MS = 300;
export const KEYFRAME_REQUEST_MIN_INTERVAL_MS = 1000;

// Output-measured encoder ladder. WebKitGTK's GStreamer-backed
// VideoEncoder holds its configured CBR only loosely (observed 6×
// overshoot: ~640 kbps emitted at a 100 kbps target, 34-108 KB
// keyframes), and WebCodecs exposes no stricter rate-control knob on
// that engine — so the configured bitrate must never be trusted as
// achieved. The sender measures actual encoded output per window and
// steps resolution/fps down until output fits the target. The window
// exceeds KEYFRAME_INTERVAL_MS so every measurement includes at least
// one keyframe (a keyframe-free window would read deceptively low).
export const LADDER_WINDOW_MS = 5000;
export const LADDER_OVERSHOOT_RATIO = 1.25;
export const LADDER_UNDERSHOOT_RATIO = 0.6;
// Consecutive headroom windows required before stepping back up —
// one quiet-scene window must not bounce the ladder.
export const LADDER_UP_STREAK = 2;

// `wireCodecToWebCodecsString` moved to `src/utils/webcodecs.ts`. It is
// a pure mapping of a wire value, and leaving it here forced
// `handlers/video.handlers.ts` to import a component module to
// configure a decoder. Re-exported so the sender and decoder keep one
// import site.
export { wireCodecToWebCodecsString } from "../../../utils/webcodecs";

// The receiver playout-buffer bounds (PLAYOUT_MIN_DELAY_MS and friends)
// moved to ./playout_buffer.ts, their only consumer. They were declared
// here while this file imported `VideoPlayoutBuffer` back out of that
// module, which is the whole of that cycle.
export const ACK_INTERVAL_MS = 1000; // measured kbps/loss feedback cadence

// Per-stream render-path latency logging (~1 Hz) to confirm the buffer sits at
// the live ceiling and isolate the decoder's internal latency per engine
// (WKWebView / WebView2 / WebKitGTK differ). Turn off once the pipeline is tuned.
export const DEBUG_VIDEO_LATENCY = true;

export interface RemoteStream {
  streamId: string;
  senderPseudonym: string;
  /** Codec this stream's decoder is configured for — from the
   *  per-frame tag. A tag change tears the decoder down. */
  codec: Codec;
  decoder: VideoDecoder;
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D | null;
  // False until the synchronous decoder.configure() lands; the playout
  // pump skips decode until then. Phase C — driven by the negotiated
  // `SessionVideoConfig` from the backend, no per-WebView probe.
  ready: boolean;
  // Reorder + jitter-absorb encoded chunks before decode (see playout_buffer.ts).
  buffer: VideoPlayoutBuffer;
  // Throttles measured-ack emission to ACK_INTERVAL_MS (performance.now ms).
  lastAckAt: number;
  // FIFO of performance.now() at each decoder.decode() call; paired with the
  // decoder's output callback to measure decode→paint (the decoder's internal
  // latency), isolated from the buffer's playout delay. Only used when
  // DEBUG_VIDEO_LATENCY is on.
  decodeStamps: number[];
  // Most recent decode→output latency in ms (diagnostic).
  lastDecodeMs: number;
  // Throttles the ~1 Hz diagnostic log (performance.now ms).
  lastDebugAt: number;
  /** performance.now() of the last decoder rebuild — a fatal WebCodecs
   *  decoder error closes the decoder permanently; recovery recreates
   *  it (cooldown-guarded) instead of keyframe-requesting a corpse. */
  lastDecoderRebuildAt: number;
  /** True after a rebuild until the first keyframe decodes — feeding a
   *  fresh decoder a delta is itself a fatal error, so the pump skips
   *  deltas while this is set. */
  awaitKeyframe: boolean;
}

// Declared once in utils/base64 — the voice recorder needs the same
// encoder. Re-exported so this module's existing importers are unchanged.
export { bytesToBase64, decodeBase64ToBytes } from "../../../utils/base64";

/** W11.4 — DM-mode random 16-byte stream id (hex). DM is 1:1, so
 *  there's no per-channel collision risk that would require the
 *  backend's deterministic derivation. */
export function randomStreamIdHex(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  let s = "";
  for (const b of bytes) s += b.toString(16).padStart(2, "0");
  return s;
}
