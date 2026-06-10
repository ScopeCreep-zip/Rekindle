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

/** `Codec` wire string → fully-specified WebCodecs codec parameter.
 *  Shared by the encoder (video_sender.ts) and decoder
 *  (useVideoCallPanel.ts) — one map, one edit per new codec. H.264 is
 *  constrained-baseline in Annex-B form: the encoder additionally sets
 *  `avc: { format: "annexb" }`, and decoders configure
 *  codec-string-only (SPS/PPS ride the bitstream; no avcC
 *  description). */
export function wireCodecToWebCodecsString(codec: Codec): string {
  switch (codec) {
    case "vp9":
      return "vp09.00.30.08";
    case "vp8":
      return "vp8";
    case "h264":
      return "avc1.42E01F";
  }
}

// Receiver playout buffer. delay = jitterEstimate × multiplier, clamped to
// [min, max]. These are *call* bounds (live), not *streaming* bounds: WebRTC's
// video jitter buffer lives at ~50–200ms and libwebrtc's low-latency start
// delay is ~30–40ms. The 500ms/×3 streaming defaults pinned delay near the
// ceiling on bursty gossip arrival and put video ~1s behind live.
export const PLAYOUT_MIN_DELAY_MS = 40; // libwebrtc low-latency start delay
export const PLAYOUT_MAX_DELAY_MS = 180; // WebRTC video jitter-buffer ceiling
export const PLAYOUT_JITTER_MULTIPLIER = 2.5;
export const PLAYOUT_MAX_FRAMES = 30; // ~2s @15fps — a runaway guard, not a buffer
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
}

export function decodeBase64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

export function bytesToBase64(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i += 1) s += String.fromCharCode(bytes[i]);
  return btoa(s);
}

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
