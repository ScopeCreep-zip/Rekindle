import { Component, For, Show } from "solid-js";
import { useVideoCallPanel, type VideoCallPanelProps } from "./video_call/useVideoCallPanel";

// Architecture §10.6 — interim video pipeline. The browser captures via
// getUserMedia (camera) or getDisplayMedia (screen), draws the live
// MediaStream to an offscreen canvas at the target framerate, builds
// `VideoFrame`s from the canvas, encodes with WebCodecs VideoEncoder
// (VP9, 480p @ 15 fps, keyframe every 2 s), hands each chunk to
// `sendVideoFrame` which fragments + MEK-encrypts + gossips. Receivers
// reassemble in `services/community/video/` and push each frame to the
// per-stream `ipc::Channel` registered in `useVideoCallPanel.ts` (Phase
// 11 Tier 1, off the shared event bus); we decode here with VideoDecoder.
// Canvas capture is
// used in lieu of `MediaStreamTrackProcessor` because that API is
// Chromium-only — WKWebView (macOS) and WebKitGTK (Linux) do not
// implement Insertable Streams. `VideoEncoder` / `VideoDecoder` are
// available in WKWebView 17.5+ and WebKitGTK 2.46+, so this canvas
// path runs on all three of our target platforms.
//
// This file is the JSX shell only — the WebCodecs send/receive pipeline,
// store mirroring, and PiP logic live in `video_call/useVideoCallPanel.ts`
// (+ `video_sender.ts` / `codec_utils.ts`).

const VideoCallPanel: Component<VideoCallPanelProps> = (props) => {
  const vm = useVideoCallPanel(props);

  return (
    <div class="video-call-panel" classList={{ "video-call-panel-hidden": !props.visible }}>
      <Show when={vm.error()}>
        <div class="search-panel-error" role="alert">{vm.error()}</div>
      </Show>
      <div class="video-call-pip-toolbar">
        <button
          type="button"
          class="video-call-pip-btn"
          title="Toggle picture-in-picture"
          aria-label="Toggle picture-in-picture"
          onClick={() => void vm.togglePictureInPicture()}
        >
          PiP
        </button>
      </div>
      <div class="video-call-grid">
        <Show when={vm.cameraOn()}>
          <div class="video-call-tile">
            <video
              ref={(el) => (vm.localCameraVideoRef.value = el)}
              autoplay
              playsinline
              muted
              class="video-call-video"
            />
            <div class="video-call-tile-label">You · camera</div>
          </div>
        </Show>
        <Show when={vm.screenOn()}>
          <div class="video-call-tile">
            <video
              ref={(el) => (vm.localScreenVideoRef.value = el)}
              autoplay
              playsinline
              muted
              class="video-call-video"
            />
            <div class="video-call-tile-label">You · screen</div>
          </div>
        </Show>
        <For each={vm.remotes()}>
          {(remote) => (
            <div class="video-call-tile">
              <div
                class="video-call-canvas"
                ref={(el) => {
                  // Move the off-DOM canvas into this slot the first time
                  // it's mounted; subsequent renders keep the same ref.
                  if (el && remote.canvas.parentElement !== el) {
                    el.appendChild(remote.canvas);
                  }
                }}
              />
              <div class="video-call-tile-label">{remote.senderPseudonym.slice(0, 8)}</div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

export default VideoCallPanel;
