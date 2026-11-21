// Wave 12 W12.7 — Picture-in-Picture. Prefers a remote tile (canvas
// bridged through a hidden <video> via canvas.captureStream); falls
// back to the local camera <video> if no remote is showing yet.
// Extracted from useVideoCallPanel.ts; plain function of the PanelCtx.

import type { PanelCtx, Ref } from "./panel_ctx";

/** The PiP toggle and the release that drops the bridge it built. */
export interface PipController {
  toggle: () => Promise<void>;
  /** Stop the bridge's canvas capture tracks, detach its `srcObject`
   *  and remove the hidden `<video>` (plan CL2): the panel's teardown
   *  calls it so nothing outlives the call. */
  release: () => void;
}

export function createPipToggle(ctx: PanelCtx): PipController {
  const pipBridgeVideo: Ref<HTMLVideoElement | null> = { value: null };
  /** Stop the bridge's current capture stream and detach it. */
  function dropBridgeStream(): void {
    const v = pipBridgeVideo.value;
    if (!v) return;
    if (v.srcObject instanceof MediaStream) {
      v.srcObject.getTracks().forEach((t) => t.stop());
    }
    v.srcObject = null;
  }
  function release(): void {
    dropBridgeStream();
    pipBridgeVideo.value?.remove();
    pipBridgeVideo.value = null;
  }
  async function toggle(): Promise<void> {
    try {
      if (document.pictureInPictureElement) {
        await document.exitPictureInPicture();
        dropBridgeStream();
        return;
      }
      // PiP shows a remote peer (every entry in remotes() is a peer now
      // — the self-view is a direct getUserMedia preview, not a stream).
      const remote = ctx.remotes()[0];
      if (
        remote &&
        typeof (remote.canvas as HTMLCanvasElement).captureStream === "function"
      ) {
        const stream = (remote.canvas as HTMLCanvasElement).captureStream(30);
        if (!pipBridgeVideo.value) {
          const v = document.createElement("video");
          v.autoplay = true;
          v.muted = true;
          v.playsInline = true;
          v.style.position = "fixed";
          v.style.opacity = "0";
          v.style.width = "1px";
          v.style.height = "1px";
          v.style.pointerEvents = "none";
          document.body.appendChild(v);
          pipBridgeVideo.value = v;
        }
        dropBridgeStream();
        pipBridgeVideo.value.srcObject = stream;
        await pipBridgeVideo.value.play().catch(() => {});
        await pipBridgeVideo.value.requestPictureInPicture();
        return;
      }
      const localVideo = ctx.localCameraVideoRef.value ?? ctx.localScreenVideoRef.value;
      if (localVideo) {
        await localVideo.requestPictureInPicture();
      }
    } catch (e) {
      console.warn("Picture-in-Picture failed:", e);
    }
  }
  return { toggle, release };
}
