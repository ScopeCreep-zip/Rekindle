/**
 * Base64 <-> byte-array conversion.
 *
 * `bytesToBase64` existed in `chat/message_input/useVoiceRecorder.ts`
 * and `voice/video_call/codec_utils.ts` with identical bodies (only the
 * accumulator was named differently). Both encode binary the frontend
 * hands to the backend, so they must agree.
 *
 * These use the `btoa`/`atob` + charCode loop rather than
 * `TextDecoder`, because the input is arbitrary binary: a UTF-8 decode
 * would mangle any byte sequence that is not valid UTF-8.
 */

/** Encode raw bytes as a standard base64 string. */
export function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 1) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

/** Decode a standard base64 string back into raw bytes. */
export function decodeBase64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}
