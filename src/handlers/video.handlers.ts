// Phase B / C — one-shot WebCodecs probe matrix. Runs at app startup
// (BuddyListWindow.onMount) and reports the WebView's real encoder +
// decoder reach to the backend. The backend's negotiator then picks a
// `SessionVideoConfig` against the actual capability set instead of the
// conservative `MediaCapabilities::interim_default()` placeholder.
//
// No fallback paths: if the WebView cannot encode or decode VP9 the
// app refuses to run video. WKWebView (macOS) and WebKitGTK (Linux)
// both ship WebCodecs VP9; WebView2 ships it too — there is no
// supported Tauri platform where this should fail. A failure means
// either a missing build dependency (GStreamer plugins on Linux) or a
// version skew, and surfacing it loudly is correct.

import { commands } from "../ipc/commands";
import type { MediaCapabilities, ScalabilityMode } from "../ipc/commands";

const VP9_CODEC = "vp09.00.30.08";
const PROBE_WIDTH = 854;
const PROBE_HEIGHT = 480;
const PROBE_FRAMERATE = 15;
const PROBE_BITRATE = 500_000;

let probeRan = false;

/// Probe `VideoEncoder.isConfigSupported` × `VideoDecoder.isConfigSupported`
/// for the {flat, L1T2} × {baseline, optimizeForLatency} matrix, then
/// report the result. Idempotent at module scope — calling more than once
/// per process is a no-op (the backend's report-handler is itself
/// idempotent, but we keep the no-op here so we never double-await the
/// probes).
export async function probeAndReportLocalVideoCapabilities(): Promise<void> {
  if (probeRan) return;
  probeRan = true;

  const [encoderBaseline, encoderL1T2, decoderBaseline, decoderOptimizeLatency] =
    await Promise.all([
      VideoEncoder.isConfigSupported({
        codec: VP9_CODEC,
        width: PROBE_WIDTH,
        height: PROBE_HEIGHT,
        framerate: PROBE_FRAMERATE,
        bitrate: PROBE_BITRATE,
      }),
      VideoEncoder.isConfigSupported({
        codec: VP9_CODEC,
        width: PROBE_WIDTH,
        height: PROBE_HEIGHT,
        framerate: PROBE_FRAMERATE,
        bitrate: PROBE_BITRATE,
        scalabilityMode: "L1T2",
      }),
      VideoDecoder.isConfigSupported({ codec: VP9_CODEC }),
      VideoDecoder.isConfigSupported({
        codec: VP9_CODEC,
        optimizeForLatency: true,
      }),
    ]);

  // Refuse to advertise VP9 if even the baseline encoder or decoder
  // can't be configured. The backend cannot pick a fallback codec —
  // there isn't one — so this is a hard failure, not a degrade. The
  // probe ran flag stays set; we never retry silently.
  if (!encoderBaseline.supported || !decoderBaseline.supported) {
    throw new Error(
      "WebView lacks VP9 WebCodecs support — video calls cannot run.",
    );
  }

  const supportedScalabilityModes: ScalabilityMode[] = [];
  if (encoderBaseline.supported) supportedScalabilityModes.push("flat");
  if (encoderL1T2.supported) supportedScalabilityModes.push("l1t2");

  // optimizeForLatency is a decoder-side hint; both the baseline AND
  // the explicit-flag probe must succeed before we tell the backend the
  // platform actually honours it. Probing only the explicit-flag form
  // (the prior implementation) misreports WebViews that accept the
  // option but ignore it.
  const supportsOptimizeForLatency = Boolean(
    decoderBaseline.supported && decoderOptimizeLatency.supported,
  );

  const caps: MediaCapabilities = {
    maxPixelCount: PROBE_WIDTH * PROBE_HEIGHT,
    maxFps: PROBE_FRAMERATE,
    codecs: ["vp9"],
    supportsOptimizeForLatency,
    supportedScalabilityModes,
  };

  await commands.reportLocalVideoCapabilities(caps);
}
