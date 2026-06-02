// Architecture §10.6 — shared constants + pure helpers for the interim
// WebCodecs video pipeline. Split out of VideoCallPanel so the sender
// (video_sender.ts) and the orchestration hook can share them without a
// circular import.

export const ENCODE_WIDTH = 854; // 480p widescreen
export const ENCODE_HEIGHT = 480;
export const ENCODE_FPS = 15;
export const KEYFRAME_INTERVAL_MS = 2000;

export interface RemoteStream {
  streamId: string;
  senderPseudonym: string;
  decoder: VideoDecoder;
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D | null;
  // Architecture §10.6 — keyframe gate: we drop deltas until a keyframe
  // initialises the decoder for this stream.
  ready: boolean;
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
