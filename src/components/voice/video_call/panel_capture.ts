// Capture controller — camera/screen getUserMedia lifecycles plus the
// Linux-native capture path (backend GStreamer + JPEG self-view).
// Extracted from useVideoCallPanel.ts; plain functions of the shared
// PanelCtx (no SolidJS owner-scoped primitives here).

import { Channel } from "@tauri-apps/api/core";
import { commands } from "../../../ipc/commands";
import type { NativePreviewFrameMsg } from "../../../ipc/commands";
import { videoSessionConfigFor } from "../../../stores/video.store";
import { decodeBase64ToBytes } from "./codec_utils";
import type { PanelCtx } from "./panel_ctx";

export interface CaptureController {
  startCamera(): Promise<void>;
  stopCamera(): Promise<void>;
  startScreen(): Promise<void>;
  stopScreen(): Promise<void>;
}

/** Capture / display constraints come from the backend-negotiated
 *  encoder config (Phase B / C). DM mode (no community context) uses
 *  the baseline 480p@15 floor — the codec pick is the sender's
 *  concern (see encoder_config.ts pickDmEncoderCodec). */
function captureConstraints(ctx: PanelCtx): {
  width: number;
  height: number;
  frameRate: number;
} {
  if (ctx.props.mode === "community") {
    const config = videoSessionConfigFor(ctx.props.communityId, ctx.props.channelId);
    if (config) {
      return {
        width: config.encoder.maxWidth,
        height: config.encoder.maxHeight,
        frameRate: config.encoder.maxFps,
      };
    }
  }
  return { width: 854, height: 480, frameRate: 15 };
}

/// Resolve the persisted camera selection against the LIVE device
/// list: exact deviceId first, then label (WebKit deviceIds are
/// origin/data-store salted and rotate across reinstalls — the
/// label is the stable key), else system default. Preferences are
/// read via IPC because Tauri windows are separate JS contexts —
/// the Settings WINDOW's store writes never reach this window's
/// `settingsState` (the old code read a copy that was always null,
/// so the saved selection silently never applied).
async function resolveSavedCamera(): Promise<string | undefined> {
  try {
    const prefs = await commands.getPreferences();
    const savedId = prefs.videoDeviceId;
    const savedLabel = prefs.videoDeviceLabel;
    if (!savedId && !savedLabel) return undefined;
    const devices = await navigator.mediaDevices.enumerateDevices();
    const cams = devices.filter((d) => d.kind === "videoinput");
    if (savedId && cams.some((d) => d.deviceId === savedId)) return savedId;
    if (savedLabel) {
      const byLabel = cams.find((d) => d.label === savedLabel);
      if (byLabel) return byLabel.deviceId;
    }
    void commands.reportMediaCaptureError(
      "camera-saved-device",
      `saved camera not in device list (id=${savedId ?? "-"}, label=${savedLabel ?? "-"}) — using default`,
    );
  } catch {
    // Preference read / enumeration unavailable — default camera.
  }
  return undefined;
}

export function createCaptureController(ctx: PanelCtx): CaptureController {
  async function startCamera(): Promise<void> {
    ctx.setError(null);
    // Backend-native capture path (capability-detected): the backend
    // owns encode to peers (GStreamer VP9), and the self view is a
    // direct getUserMedia preview (no webview encoder, no loopback).
    // Re-query on a cold cache: a click racing the onMount probe must
    // not fall back to the webview encoder on a native-capable box
    // (the probe is OnceLock-cached backend-side — this is cheap).
    if (!ctx.nativeCaptureAvailable() && ctx.props.mode === "community") {
      ctx.setNativeCaptureAvailable(
        await commands.nativeVideoCaptureAvailable().catch(() => false),
      );
    }
    if (ctx.nativeCaptureAvailable() && ctx.props.mode === "community") {
      // Self-view canvas + preview channel, registered BEFORE the native
      // session starts so the first JPEG stills aren't dropped. The
      // backend captures ONCE (v4l2src) and tees: VP9 to peers + JPEG
      // stills here. No second getUserMedia consumer, no loopback.
      const canvas = document.createElement("canvas");
      canvas.width = 320;
      canvas.height = 180;
      const canvasCtx = canvas.getContext("2d");
      const ch = new Channel<NativePreviewFrameMsg>();
      ch.onmessage = (msg) => {
        if (!canvasCtx || ctx.nativePreviewDecoding) return;
        ctx.nativePreviewDecoding = true;
        const bytes = decodeBase64ToBytes(msg.jpegB64);
        const blob = new Blob([bytes as BlobPart], { type: "image/jpeg" });
        void createImageBitmap(blob)
          .then((bmp) => {
            canvasCtx.drawImage(bmp, 0, 0, canvas.width, canvas.height);
            bmp.close();
          })
          .catch((e: unknown) => {
            // A corrupt still just doesn't paint; the next is a fresh
            // full JPEG (intra-only, no reference chain). Surface it so a
            // persistent decode failure is visible in the terminal log.
            const m = e instanceof Error ? e.message : String(e);
            void commands.reportMediaCaptureError(
              "camera-native-preview",
              `self-view createImageBitmap failed: ${m}`,
            );
          })
          .finally(() => {
            ctx.nativePreviewDecoding = false;
          });
      };
      ctx.nativePreviewChannel = ch;
      void commands.registerNativePreviewChannel(ch);
      ctx.setNativeSelfCanvas(canvas);
      try {
        const prefs = await commands.getPreferences();
        ctx.nativeStreamId = await commands.startNativeVideo(
          ctx.props.communityId,
          ctx.props.channelId,
          "camera",
          prefs.videoDeviceLabel ?? null,
        );
        ctx.setCameraOn(true);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        ctx.nativeStreamId = null;
        void commands.unregisterNativePreviewChannel();
        ctx.nativePreviewChannel = null;
        ctx.setNativeSelfCanvas(null);
        ctx.setError(`Camera failed: ${msg}`);
        void commands.reportMediaCaptureError("camera-native", msg);
      }
      return;
    }
    const { width, height, frameRate } = captureConstraints(ctx);
    const open = (deviceId: string | undefined) =>
      navigator.mediaDevices.getUserMedia({
        video: {
          deviceId: deviceId ? { exact: deviceId } : undefined,
          width,
          height,
          frameRate,
        },
        audio: false,
      });
    try {
      // Resolved persisted selection (id → label → default); the
      // unpinned retry below stays as the safety net for a device
      // that vanishes between resolution and open.
      const savedId = await resolveSavedCamera();
      let stream: MediaStream;
      try {
        stream = await open(savedId ?? undefined);
      } catch (first) {
        if (!savedId) throw first;
        const firstMsg = first instanceof Error ? first.message : String(first);
        void commands.reportMediaCaptureError(
          "camera-saved-device",
          `saved camera unavailable (${firstMsg}) — retrying default`,
        );
        stream = await open(undefined);
        ctx.setError("Saved camera unavailable — using default camera");
      }
      // Capture hygiene (all platforms): the delivered camera mode can
      // differ from the constraints above — drivers commonly hand back
      // the full-native mode (noisy MJPEG, different aspect) and let
      // the UA scale in software. Nudge the track toward the encode
      // shape, then LOG what was actually delivered — every mismatch
      // here turns into encoder entropy the bitrate budget pays for,
      // and it was invisible until now.
      const track = stream.getVideoTracks()[0];
      if (track) {
        try {
          await track.applyConstraints({ width, height, frameRate });
        } catch {
          // Best-effort: a camera that can't hit the shape still works —
          // the sender's aspect-correct draw absorbs the difference.
        }
        const s = track.getSettings();
        void commands.reportMediaCaptureError(
          "camera-settings",
          `delivered ${s.width}x${s.height}@${s.frameRate ?? "?"}fps ` +
            `(wanted ${width}x${height}@${frameRate})`,
        );
      }
      ctx.cameraStream = stream;
      ctx.setCameraCapture(stream);
      if (ctx.localCameraVideoRef.value) {
        ctx.localCameraVideoRef.value.srcObject = stream;
      }
      // Capture + local preview only — the egress effect attaches the
      // sender when (and only when) the media-ready gate is open, so a
      // solo member sees their own tile immediately and sending starts
      // the moment a peer connects.
      ctx.setCameraOn(true);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      ctx.setError(`Camera failed: ${msg}`);
      void commands.reportMediaCaptureError("camera", msg);
      ctx.cameraStream?.getTracks().forEach((t) => t.stop());
      ctx.cameraStream = null;
      ctx.setCameraCapture(null);
    }
  }

  async function stopCamera(): Promise<void> {
    if (ctx.nativeStreamId !== null) {
      ctx.nativeStreamId = null;
      try {
        await commands.stopNativeVideo();
      } catch (e) {
        console.error("stop_native_video failed:", e);
      }
      // Tear down the self-view preview channel + canvas.
      void commands.unregisterNativePreviewChannel();
      ctx.nativePreviewChannel = null;
      ctx.setNativeSelfCanvas(null);
      ctx.setCameraOn(false);
      return;
    }
    ctx.sender.stop("camera");
    ctx.cameraStream?.getTracks().forEach((t) => t.stop());
    ctx.cameraStream = null;
    ctx.setCameraCapture(null);
    if (ctx.localCameraVideoRef.value) {
      ctx.localCameraVideoRef.value.srcObject = null;
    }
    ctx.setCameraOn(false);
  }

  async function startScreen(): Promise<void> {
    ctx.setError(null);
    try {
      const { frameRate } = captureConstraints(ctx);
      const stream = await navigator.mediaDevices.getDisplayMedia({
        video: { frameRate },
        audio: false,
      });
      ctx.screenStream = stream;
      ctx.setScreenCapture(stream);
      if (ctx.localScreenVideoRef.value) {
        ctx.localScreenVideoRef.value.srcObject = stream;
      }
      // Auto-stop encoder when the user clicks "Stop sharing" in the
      // browser's screen-share controls.
      stream.getVideoTracks()[0]?.addEventListener("ended", () => {
        void stopScreen();
      });
      // Capture + preview only — sender attaches via the egress effect.
      ctx.setScreenOn(true);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      ctx.setError(`Screen share failed: ${msg}`);
      void commands.reportMediaCaptureError("screen", msg);
      ctx.screenStream?.getTracks().forEach((t) => t.stop());
      ctx.screenStream = null;
      ctx.setScreenCapture(null);
    }
  }

  async function stopScreen(): Promise<void> {
    ctx.sender.stop("screen");
    ctx.screenStream?.getTracks().forEach((t) => t.stop());
    ctx.screenStream = null;
    ctx.setScreenCapture(null);
    if (ctx.localScreenVideoRef.value) {
      ctx.localScreenVideoRef.value.srcObject = null;
    }
    ctx.setScreenOn(false);
  }

  return { startCamera, stopCamera, startScreen, stopScreen };
}
