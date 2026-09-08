// Wire codec tag → WebCodecs codec parameter string.
//
// A pure mapping of an IPC wire value, so it belongs in the utils leaf
// rather than inside a component module. It lived in
// `components/voice/video_call/codec_utils.ts`, which made
// `handlers/video.handlers.ts` — the module that configures decoders
// from incoming frames — import a component to do it.

import type { Codec } from "../ipc/commands/types_sync";

/** VP9 profile 0, level 3.0, 8-bit — the negotiated default.
 *
 *  H.264 is `avc1.42E01F`: constrained-baseline in Annex-B form: the
 *  encoder additionally sets `avc: { format: "annexb" }`, and decoders
 *  configure codec-string-only (SPS/PPS ride the bitstream; no avcC
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
