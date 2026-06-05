// Architecture §10.6 — shared constants + pure helpers for the interim
// WebCodecs video pipeline. Split out of VideoCallPanel so the sender
// (video_sender.ts) and the orchestration hook can share them without a
// circular import.
import type { VideoPlayoutBuffer } from "./playout_buffer";

export const ENCODE_WIDTH = 854; // 480p widescreen
export const ENCODE_HEIGHT = 480;
export const ENCODE_FPS = 15;
export const KEYFRAME_INTERVAL_MS = 2000;

// Receiver playout buffer (videocall-codecs reference values, adapted for the
// gossip mesh). delay = jitterEstimate × multiplier, clamped to [min, max].
export const PLAYOUT_MIN_DELAY_MS = 60; // floor; mesh jitter rarely below this
export const PLAYOUT_MAX_DELAY_MS = 500;
export const PLAYOUT_JITTER_MULTIPLIER = 3.0;
export const PLAYOUT_MAX_FRAMES = 200;
export const ACK_INTERVAL_MS = 1000; // measured kbps/loss feedback cadence

export interface RemoteStream {
  streamId: string;
  senderPseudonym: string;
  decoder: VideoDecoder;
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D | null;
  // Architecture §10.6 — keyframe gate: we drop deltas until a keyframe
  // initialises the decoder for this stream.
  ready: boolean;
  // Reorder + jitter-absorb encoded chunks before decode (see playout_buffer.ts).
  buffer: VideoPlayoutBuffer;
  // Throttles measured-ack emission to ACK_INTERVAL_MS (performance.now ms).
  lastAckAt: number;
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
