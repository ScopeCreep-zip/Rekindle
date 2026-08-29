// Shared mutable context for the video-call panel's extracted modules
// (panel_decode.ts, panel_capture.ts, panel_pip.ts). One object, created
// by useVideoCallPanel and threaded explicitly — the extracted code is
// plain functions of this context, holding no SolidJS owner-scoped
// primitives of its own (signals are created in the hook; effects and
// lifecycle callbacks never leave it).

import type { Channel } from "@tauri-apps/api/core";
import type { NativePreviewFrameMsg } from "../../../ipc/commands";
import type { RemoteStream } from "./codec_utils";
import type { VideoSender } from "./sender_types";
import type { VideoCallPanelProps } from "./useVideoCallPanel";

/** Mutable ref cell (the hook's `{ value: … }` binder convention). */
export interface Ref<T> {
  value: T;
}

export interface PanelCtx {
  props: VideoCallPanelProps;
  sender: VideoSender;

  // Signals created by the hook (accessor/setter pairs — plain closures,
  // safe to call from anywhere).
  remotes: () => RemoteStream[];
  setRemotes: (update: (prev: RemoteStream[]) => RemoteStream[]) => void;
  setError: (msg: string | null) => void;
  setCameraOn: (on: boolean) => void;
  setScreenOn: (on: boolean) => void;
  setCameraCapture: (s: MediaStream | null) => void;
  setScreenCapture: (s: MediaStream | null) => void;
  nativeCaptureAvailable: () => boolean;
  setNativeCaptureAvailable: (v: boolean) => void;
  setNativeSelfCanvas: (c: HTMLCanvasElement | null) => void;

  // Plain mutable state shared across modules and the hook's lifecycle
  // callbacks.
  cameraStream: MediaStream | null;
  screenStream: MediaStream | null;
  nativeStreamId: string | null;
  nativePreviewChannel: Channel<NativePreviewFrameMsg> | null;
  nativePreviewDecoding: boolean;

  localCameraVideoRef: Ref<HTMLVideoElement | undefined>;
  localScreenVideoRef: Ref<HTMLVideoElement | undefined>;

  /** Re-arms the playout clock (owned by the hook's scheduling trio). */
  schedulePlayout: () => void;
}
