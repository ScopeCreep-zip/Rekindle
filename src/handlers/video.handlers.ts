// Phase 4 — one-shot WebCodecs probe matrix, per-codec × per-direction.
// Runs lazily when the first voice/call session is joined
// (handleJoinVoice) and reports the WebView's real encoder + decoder
// reach to the backend. The backend's negotiator then picks the LOCAL
// encoder codec against every peer's decode set (per-node pick — a Mac
// sending H.264 while a Pop!_OS peer sends VP9 is the correct steady
// state); caps that land after LocalJoined recompute + re-emit the
// config, so lazy reporting is fully supported.
//
// The probe must NEVER run on the login path. WebKitGTK 2.52.3 (the
// 2026-05 Ubuntu/Pop!_OS 24.04 security backport, built against
// GStreamer 1.24) has an initialization-order bug: calling
// `VideoEncoder.isConfigSupported` as the FIRST media API in a fresh
// web process registers webkit's encoder element before GStreamer is
// initialized, corrupting GType registration and ABORTING the whole
// WebKitWebProcess ("GStreamer:ERROR gst_register_core_elements") —
// which killed the UI right after login when this probe ran from
// BuddyListWindow.onMount. Touching any other media API first
// (enumerateDevices) initializes GStreamer properly; see warmup below.
//
// Every probe is INDIVIDUALLY try/caught: Safari's isConfigSupported
// may REJECT instead of resolving `{ supported: false }` for codecs it
// doesn't recognize (w3c/webcodecs#744) — one unrecognized codec must
// never kill the whole matrix. The only fatal case is BOTH lists
// coming back empty (no usable WebCodecs at all); a decode-only
// platform (empty encode list) reports and proceeds — the user can
// watch video without sending.

import { commands } from "../ipc/commands";
import type { Codec, MediaCapabilities, ScalabilityMode } from "../ipc/commands";
import { wireCodecToWebCodecsString } from "../components/voice/video_call/codec_utils";

const PROBE_WIDTH = 854;
const PROBE_HEIGHT = 480;
const PROBE_FRAMERATE = 15;
const PROBE_BITRATE = 500_000;

/** Preference-ordered probe matrix. VP9 first (quality), VP8 second
 *  (the 2026 software floor — libvpx everywhere), H.264
 *  constrained-baseline last (the Mac↔Linux hardware bridge). The
 *  encoder probe for h264 sets `avc: { format: "annexb" }` so SPS/PPS
 *  ride the bitstream and decoders configure codec-string-only. */
function codecProbes(): {
  codec: Codec;
  encoderConfig: VideoEncoderConfig;
  decoderConfig: VideoDecoderConfig;
}[] {
  return (["vp9", "vp8", "h264"] as Codec[]).map((codec) => {
    const codecString = wireCodecToWebCodecsString(codec);
    const encoderConfig: VideoEncoderConfig = {
      codec: codecString,
      width: PROBE_WIDTH,
      height: PROBE_HEIGHT,
      framerate: PROBE_FRAMERATE,
      bitrate: PROBE_BITRATE,
      ...(codec === "h264" ? { avc: { format: "annexb" as const } } : {}),
    };
    return { codec, encoderConfig, decoderConfig: { codec: codecString } };
  });
}

/** Run one isConfigSupported probe, mapping rejection to `false`
 *  (Safari rejects on unrecognized codecs instead of resolving
 *  `{ supported: false }` — w3c/webcodecs#744). */
async function probeSupported(
  fn: () => Promise<{ supported?: boolean }>,
): Promise<boolean> {
  try {
    return (await fn()).supported === true;
  } catch {
    return false;
  }
}

let probeRan = false;
let cachedCaps: MediaCapabilities | null = null;

/** The most recent probe result, or `null` before the probe ran. The
 *  DM sender intersects `encodeCodecs` against the DM peer's
 *  advertised decode list (community mode reads the backend-emitted
 *  `SessionVideoConfig` instead). */
export function localVideoCapabilities(): MediaCapabilities | null {
  return cachedCaps;
}

/// Probe encode + decode support for every shipped codec, plus the
/// VP9-only L1T2 scalability probe and the decoder `optimizeForLatency`
/// hint, then report the result. Idempotent at module scope — calling
/// more than once per process is a no-op (the backend's report-handler
/// is itself idempotent, but we keep the no-op here so we never
/// double-await the probes).
export async function probeAndReportLocalVideoCapabilities(): Promise<void> {
  if (probeRan) return;
  probeRan = true;

  // Warm the WebView's media stack before the first WebCodecs call.
  // On WebKitGTK 2.52.3 + GStreamer 1.24 (Ubuntu/Pop!_OS 24.04
  // security backport) a cold `isConfigSupported` aborts the web
  // process (init-order bug, see header). enumerateDevices is
  // side-effect-free everywhere else (no permission prompt, labels
  // anonymized) and forces GStreamer to initialize first.
  try {
    await navigator.mediaDevices.enumerateDevices();
  } catch {
    // No media devices / API unavailable — the probes below decide
    // whether video is possible; warming is best-effort.
  }

  const encodeCodecs: Codec[] = [];
  const decodeCodecs: Codec[] = [];
  for (const probe of codecProbes()) {
    const [enc, dec] = await Promise.all([
      probeSupported(() => VideoEncoder.isConfigSupported(probe.encoderConfig)),
      probeSupported(() => VideoDecoder.isConfigSupported(probe.decoderConfig)),
    ]);
    if (enc) encodeCodecs.push(probe.codec);
    if (dec) decodeCodecs.push(probe.codec);
  }

  // The only fatal shape: no encoder AND no decoder. A decode-only
  // platform still reports (watch-without-send is a supported mode —
  // the backend logs it instead of emitting an incompatibility).
  if (encodeCodecs.length === 0 && decodeCodecs.length === 0) {
    throw new Error(
      "WebView has no usable WebCodecs video support — video calls cannot run.",
    );
  }

  // L1T2 is probed against the VP9 encoder only — the negotiator
  // forces Flat for every other codec, keeping `configure()` valid on
  // WebKit builds that reject scalabilityMode outside VP9.
  const supportedScalabilityModes: ScalabilityMode[] = ["flat"];
  if (encodeCodecs.includes("vp9")) {
    const l1t2 = await probeSupported(() =>
      VideoEncoder.isConfigSupported({
        codec: wireCodecToWebCodecsString("vp9"),
        width: PROBE_WIDTH,
        height: PROBE_HEIGHT,
        framerate: PROBE_FRAMERATE,
        bitrate: PROBE_BITRATE,
        scalabilityMode: "L1T2",
      }),
    );
    if (l1t2) supportedScalabilityModes.push("l1t2");
  }

  // optimizeForLatency is a decoder-side hint, probed against our
  // strongest decodable codec. The baseline form already passed (the
  // codec is in decodeCodecs), so only the explicit-flag form remains
  // — both passing is what "the platform actually honours it" means.
  let supportsOptimizeForLatency = false;
  const firstDecode = decodeCodecs[0];
  if (firstDecode !== undefined) {
    supportsOptimizeForLatency = await probeSupported(() =>
      VideoDecoder.isConfigSupported({
        codec: wireCodecToWebCodecsString(firstDecode),
        optimizeForLatency: true,
      }),
    );
  }

  const caps: MediaCapabilities = {
    maxPixelCount: PROBE_WIDTH * PROBE_HEIGHT,
    maxFps: PROBE_FRAMERATE,
    encodeCodecs,
    decodeCodecs,
    supportsOptimizeForLatency,
    supportedScalabilityModes,
  };
  cachedCaps = caps;

  await commands.reportLocalVideoCapabilities(caps);
}
