// Architecture §10.6 — shared constants + pure helpers for the interim
// WebCodecs video pipeline. Split out of VideoCallPanel so the sender
// (video_sender.ts) and the orchestration hook can share them without a
// circular import.
import type { VideoPlayoutBuffer } from "./playout_buffer";

export const ENCODE_WIDTH = 854; // 480p widescreen
export const ENCODE_HEIGHT = 480;
export const ENCODE_FPS = 15;
export const KEYFRAME_INTERVAL_MS = 2000;

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
  decoder: VideoDecoder;
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D | null;
  // False until the async optimizeForLatency probe + decoder.configure() lands;
  // the playout pump skips decode until then (a few ms on the first stream).
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
