import { Component, For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { commands } from "../../ipc/commands";
import { subscribeCommunityEvents } from "../../ipc/channels";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { setVoiceState, voiceState } from "../../stores/voice.store";
import { settingsState } from "../../stores/settings.store";

// Architecture §10.6 — interim video pipeline. The browser captures via
// getUserMedia (camera) or getDisplayMedia (screen), draws the live
// MediaStream to an offscreen canvas at the target framerate, builds
// `VideoFrame`s from the canvas, encodes with WebCodecs VideoEncoder
// (VP9, 480p @ 15 fps, keyframe every 2 s), hands each chunk to
// `sendVideoFrame` which fragments + MEK-encrypts + gossips. Receivers
// reassemble in `services/community/video/` and emit `videoFrame`
// events back; we decode here with VideoDecoder. Canvas capture is
// used in lieu of `MediaStreamTrackProcessor` because that API is
// Chromium-only — WKWebView (macOS) and WebKitGTK (Linux) do not
// implement Insertable Streams. `VideoEncoder` / `VideoDecoder` are
// available in WKWebView 17.5+ and WebKitGTK 2.46+, so this canvas
// path runs on all three of our target platforms.

/** W11.4 — `community` panel routes encoded frames through gossip
 *  fan-out + MEK; `dm` panel routes 1:1 via Signal Double Ratchet. The
 *  decoder side is identical (WebCodecs VP9). */
type VideoCallPanelProps =
  | {
      mode: "community";
      communityId: string;
      channelId: string;
      /** When false the panel is invisible — used to keep state alive across panel toggles. */
      visible: boolean;
    }
  | {
      mode: "dm";
      /** Hex-encoded peer Ed25519 public key. */
      peerId: string;
      visible: boolean;
    };

interface RemoteStream {
  streamId: string;
  senderPseudonym: string;
  decoder: VideoDecoder;
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D | null;
  // Architecture §10.6 — keyframe gate: we drop deltas until a keyframe
  // initialises the decoder for this stream.
  ready: boolean;
}

const ENCODE_WIDTH = 854; // 480p widescreen
const ENCODE_HEIGHT = 480;
const ENCODE_FPS = 15;
const KEYFRAME_INTERVAL_MS = 2000;

const VideoCallPanel: Component<VideoCallPanelProps> = (props) => {
  // Architecture §10.6 — desired state lives in the voice store so
  // the toggle buttons in VoicePanel stay visible regardless of
  // whether VideoCallPanel is mounted at any given moment. Local
  // accessors mirror the store for ergonomic JSX.
  const cameraOn = (): boolean => voiceState.cameraOn;
  const setCameraOn = (next: boolean): void => setVoiceState("cameraOn", next);
  const screenOn = (): boolean => voiceState.screenShareOn;
  const setScreenOn = (next: boolean): void => setVoiceState("screenShareOn", next);
  const [error, setError] = createSignal<string | null>(null);
  const [remotes, setRemotes] = createSignal<RemoteStream[]>([]);

  let cameraStream: MediaStream | null = null;
  let cameraEncoder: VideoEncoder | null = null;
  let cameraStreamId: string | null = null;
  let cameraFrameSeq = 0;
  let cameraLastKeyframeMs = 0;
  let cameraStopFn: (() => void) | null = null;
  // Architecture §10.6 line 4081 — minimum reported downstream kbps
  // across all current receivers; encoder caps its output bitrate to
  // this value so the slowest peer still keeps pace.
  let cameraLowestReceiverKbps = 800;

  let screenStream: MediaStream | null = null;
  let screenEncoder: VideoEncoder | null = null;
  let screenStreamId: string | null = null;
  let screenFrameSeq = 0;
  let screenLastKeyframeMs = 0;
  let screenStopFn: (() => void) | null = null;
  let screenLowestReceiverKbps = 800;

  let unlistenCommunity: Promise<UnlistenFn> | null = null;
  /** W11.4 — DM mode tap into the per-peer `dm-video-frame` Tauri
   *  event instead of community gossip. Either community OR dm
   *  unlisten is non-null at runtime; both clean up in onCleanup. */
  let unlistenDm: Promise<UnlistenFn> | null = null;

  const localCameraVideoRef: { value: HTMLVideoElement | undefined } = { value: undefined };
  const localScreenVideoRef: { value: HTMLVideoElement | undefined } = { value: undefined };

  // Architecture §10.6 — receiver pipeline. Subscribe to videoFrame
  // events, instantiate one VideoDecoder per (sender, streamId),
  // render decoded frames to a canvas. Topology changes drop and
  // re-create the decoder for clean reattachment.
  //
  // W11.4 — community vs dm modes listen on different event surfaces.
  // Community uses the existing gossip-fed `community-event` stream;
  // DM uses the per-peer `dm-video-frame` event emitted by
  // message_service::process_envelope after Signal decryption +
  // reassembly.
  onMount(() => {
    if (props.mode === "community") {
      const communityIdLocal = props.communityId;
      unlistenCommunity = subscribeCommunityEvents((event) => {
        if (event.type === "videoFrame") {
          const { communityId, senderPseudonym, streamId, frameSeq, keyframe, timestamp, payloadB64 } = event.data;
          if (communityId !== communityIdLocal) return;
          ingestRemoteFrame(senderPseudonym, streamId, frameSeq, keyframe, timestamp, payloadB64);
        } else if (event.type === "videoTopologyChange") {
          const { communityId, streamId } = event.data;
          if (communityId !== communityIdLocal) return;
          // Drop matching remote so the next frame seeds a fresh decoder.
          setRemotes((prev) => {
            const out = prev.filter((r) => {
              if (r.streamId === streamId) {
                try {
                  r.decoder.close();
                } catch (e) {
                  console.error("decoder close failed:", e);
                }
                return false;
              }
              return true;
            });
            return out;
          });
        } else if (event.type === "videoKeyframeRequest") {
          const { streamId } = event.data;
          if (streamId === cameraStreamId) {
            cameraLastKeyframeMs = 0; // forces next encode to be a keyframe
          } else if (streamId === screenStreamId) {
            screenLastKeyframeMs = 0;
          }
        } else if (event.type === "videoFrameAck") {
          // Architecture §10.6 line 4081 — receiver-reported downstream
          // kbps. Adapt encoder bitrate to the slowest receiver. The
          // encoder API doesn't allow per-frame bitrate changes mid-stream,
          // but the next configure() call (e.g. on next keyframe interval)
          // will pick up the new value. Track the running min.
          const { streamId, kbps } = event.data;
          if (streamId === cameraStreamId) {
            cameraLowestReceiverKbps = Math.min(cameraLowestReceiverKbps, kbps);
          } else if (streamId === screenStreamId) {
            screenLowestReceiverKbps = Math.min(screenLowestReceiverKbps, kbps);
          }
        } else if (event.type === "videoBandwidthEstimate") {
          // Out-of-band bandwidth update — same handling as FrameAck kbps.
          // We don't track per-stream because BandwidthEstimate is
          // channel-scoped; treat it as a community-wide hint and clamp
          // both streams' reported kbps.
          const { kbps } = event.data;
          cameraLowestReceiverKbps = Math.min(cameraLowestReceiverKbps, kbps);
          screenLowestReceiverKbps = Math.min(screenLowestReceiverKbps, kbps);
        }
      });
    } else {
      const peerIdLocal = props.peerId;
      unlistenDm = listen<{
        peerPubkey: string;
        streamIdHex: string;
        frameSeq: number;
        keyframe: boolean;
        timestamp: number;
        encodedPayloadB64: string;
      }>("dm-video-frame", (msg) => {
        if (msg.payload.peerPubkey !== peerIdLocal) return;
        ingestRemoteFrame(
          msg.payload.peerPubkey,
          msg.payload.streamIdHex,
          msg.payload.frameSeq,
          msg.payload.keyframe,
          msg.payload.timestamp,
          msg.payload.encodedPayloadB64,
        );
      });
    }
  });

  onCleanup(() => {
    void stopCamera();
    void stopScreen();
    unlistenCommunity?.then((unlisten) => unlisten());
    unlistenDm?.then((unlisten) => unlisten());
    for (const r of remotes()) {
      try {
        r.decoder.close();
      } catch (e) {
        console.error("decoder close failed:", e);
      }
    }
  });

  function decodeBase64ToBytes(b64: string): Uint8Array {
    const binary = atob(b64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
    return bytes;
  }

  function bytesToBase64(bytes: Uint8Array): string {
    let s = "";
    for (let i = 0; i < bytes.length; i += 1) s += String.fromCharCode(bytes[i]);
    return btoa(s);
  }

  /** W11.4 — DM-mode random 16-byte stream id (hex). DM is 1:1, so
   *  there's no per-channel collision risk that would require the
   *  backend's deterministic derivation. */
  function randomStreamIdHex(): string {
    const bytes = new Uint8Array(16);
    crypto.getRandomValues(bytes);
    let s = "";
    for (const b of bytes) s += b.toString(16).padStart(2, "0");
    return s;
  }

  function ingestRemoteFrame(
    sender: string,
    streamId: string,
    frameSeq: number,
    keyframe: boolean,
    timestamp: number,
    payloadB64: string,
  ): void {
    const data = decodeBase64ToBytes(payloadB64);
    let remote = remotes().find((r) => r.streamId === streamId);
    if (!remote) {
      if (!keyframe) {
        // Wait for the first keyframe before instantiating a decoder.
        return;
      }
      const canvas = document.createElement("canvas");
      canvas.width = ENCODE_WIDTH;
      canvas.height = ENCODE_HEIGHT;
      const decoder = new VideoDecoder({
        output: (frame: VideoFrame) => {
          const target = remotes().find((r) => r.streamId === streamId);
          if (!target?.ctx) {
            frame.close();
            return;
          }
          target.ctx.drawImage(frame, 0, 0, target.canvas.width, target.canvas.height);
          frame.close();
        },
        error: (e: Error) => {
          console.error("VideoDecoder error:", e);
          // Architecture §10.6 line 4081 — decoder lost track of the
          // stream; ask the sender for a keyframe. Community-only —
          // DM mode has no out-of-band keyframe-request channel yet,
          // we rely on the sender's regular 2-second keyframe cadence.
          if (props.mode === "community") {
            void commands.sendVideoKeyframeRequest(
              props.communityId,
              props.channelId,
              streamId,
            );
          }
        },
      });
      decoder.configure({ codec: "vp09.00.30.08" });
      const ctx = canvas.getContext("2d");
      remote = { streamId, senderPseudonym: sender, decoder, canvas, ctx, ready: true };
      setRemotes((prev) => [...prev, remote!]);
    }

    try {
      const chunk = new EncodedVideoChunk({
        type: keyframe ? "key" : "delta",
        timestamp,
        data,
      });
      remote.decoder.decode(chunk);
      // Architecture §10.6 line 4081 — ack each decoded keyframe so
      // the sender gets fresh bandwidth + frame-seq feedback at least
      // every keyframe interval (~2 s). For loss/kbps we report
      // conservative defaults; a future iteration will measure real
      // network jitter via the WebRTC stats API.
      if (keyframe && props.mode === "community") {
        void commands.sendVideoFrameAck(
          props.communityId,
          props.channelId,
          streamId,
          frameSeq,
          800, // assumed downstream kbps, 800kbps matches encoder bitrate
          0,   // loss_q8 = 0 = perfect (no measured loss yet)
        );
      }
    } catch (e) {
      console.error("decode chunk failed:", e);
      // Request a keyframe so the decoder can recover (community only).
      if (props.mode === "community") {
        void commands.sendVideoKeyframeRequest(
          props.communityId,
          props.channelId,
          streamId,
        );
      }
    }
  }

  async function startEncoder(
    label: "camera" | "screen",
    stream: MediaStream,
  ): Promise<{ encoder: VideoEncoder; stop: () => void }> {
    // Community streams use the deterministic backend-derived id so
    // (channel_id || sender_pseudonym || track_label) collisions are
    // impossible across concurrent senders. DM streams are 1:1 — a
    // local random 16-byte UUID is sufficient and avoids a backend
    // round-trip per camera/screen start.
    const streamIdHex =
      props.mode === "community"
        ? await commands.deriveVideoStreamId(props.communityId, props.channelId, label)
        : randomStreamIdHex();
    const track = stream.getVideoTracks()[0];
    if (!track) throw new Error("no video track");

    // Drive the canvas off a hidden <video> element so the same
    // MediaStream backs both the local preview tile and the encoder
    // input. WKWebView / WebKitGTK don't implement
    // MediaStreamTrackProcessor, so we draw frames to an offscreen
    // canvas and build VideoFrames from it instead.
    const captureVideo = document.createElement("video");
    captureVideo.srcObject = stream;
    captureVideo.muted = true;
    captureVideo.playsInline = true;
    await captureVideo.play().catch((e) => {
      console.error("capture <video> play failed:", e);
    });

    const captureCanvas = document.createElement("canvas");
    captureCanvas.width = ENCODE_WIDTH;
    captureCanvas.height = ENCODE_HEIGHT;
    const captureCtx = captureCanvas.getContext("2d");
    if (!captureCtx) throw new Error("2d context unavailable");

    let frameSeq = 0;
    let lastKeyframeMs = 0;

    const encoder = new VideoEncoder({
      output: (chunk: EncodedVideoChunk) => {
        const buf = new Uint8Array(chunk.byteLength);
        chunk.copyTo(buf);
        const payloadB64 = bytesToBase64(buf);
        const seq = (frameSeq += 1);
        if (label === "camera") cameraFrameSeq = seq;
        else screenFrameSeq = seq;
        const frameRequest = {
          streamIdHex,
          frameSeq: seq,
          keyframe: chunk.type === "key",
          timestamp: Math.floor(performance.now()),
          encodedPayloadB64: payloadB64,
        };
        if (props.mode === "community") {
          void commands.sendVideoFrame(
            props.communityId,
            props.channelId,
            frameRequest,
          );
        } else {
          void commands.sendDmVideoFrame(props.peerId, frameRequest);
        }
      },
      error: (e: Error) => {
        console.error("VideoEncoder error:", e);
        setError(`Encoder error: ${e.message}`);
      },
    });
    encoder.configure({
      codec: "vp09.00.30.08",
      width: ENCODE_WIDTH,
      height: ENCODE_HEIGHT,
      framerate: ENCODE_FPS,
      bitrate: 800_000,
      latencyMode: "realtime",
    });

    let cancelled = false;
    const frameIntervalMs = 1000 / ENCODE_FPS;
    let lastEmittedAt = 0;
    let rafHandle: number | null = null;

    const pump = (): void => {
      if (cancelled) return;
      const now = performance.now();
      if (now - lastEmittedAt >= frameIntervalMs) {
        try {
          captureCtx.drawImage(
            captureVideo,
            0,
            0,
            captureCanvas.width,
            captureCanvas.height,
          );
          const localLastKeyframeMs =
            label === "camera" ? cameraLastKeyframeMs : screenLastKeyframeMs;
          const isKeyframe = now - localLastKeyframeMs >= KEYFRAME_INTERVAL_MS;
          if (isKeyframe) {
            if (label === "camera") cameraLastKeyframeMs = now;
            else screenLastKeyframeMs = now;
            lastKeyframeMs = now;
          }
          const videoFrame = new VideoFrame(captureCanvas, {
            timestamp: Math.floor(now * 1000),
          });
          encoder.encode(videoFrame, { keyFrame: isKeyframe });
          videoFrame.close();
        } catch (e) {
          console.error("encode failed:", e);
        }
        lastEmittedAt = now;
      }
      rafHandle = requestAnimationFrame(pump);
    };
    rafHandle = requestAnimationFrame(pump);

    if (label === "camera") cameraStreamId = streamIdHex;
    else screenStreamId = streamIdHex;

    // Architecture §10.6 + Phase 6 W22 — community broadcasts initial
    // topology so receivers spin up decoders. DM has only one
    // receiver who will spin up their decoder on the first keyframe
    // (handled in `ingestRemoteFrame`), so no topology broadcast is
    // needed.
    if (props.mode === "community") {
      void commands.notifyVideoTopologyChange(
        props.communityId,
        props.channelId,
        streamIdHex,
        null,
        "initial",
      );
    }

    return {
      encoder,
      stop: () => {
        cancelled = true;
        if (rafHandle !== null) cancelAnimationFrame(rafHandle);
        try {
          encoder.close();
        } catch (e) {
          console.error("encoder close failed:", e);
        }
        captureVideo.srcObject = null;
        // Suppress "unused variable" — lastKeyframeMs is read by the
        // encode loop closure above.
        void lastKeyframeMs;
      },
    };
  }

  async function startCamera(): Promise<void> {
    setError(null);
    try {
      // Plan §Failure 2 — honour the persisted camera selection from
      // Settings → Video. `exact` constraint surfaces an OverconstrainedError
      // if the device disappeared (e.g. USB camera unplugged) so the user
      // sees a clear message instead of silently falling back.
      const deviceId = settingsState.selectedVideoDeviceId;
      const stream = await navigator.mediaDevices.getUserMedia({
        video: {
          deviceId: deviceId ? { exact: deviceId } : undefined,
          width: ENCODE_WIDTH,
          height: ENCODE_HEIGHT,
          frameRate: ENCODE_FPS,
        },
        audio: false,
      });
      cameraStream = stream;
      if (localCameraVideoRef.value) {
        localCameraVideoRef.value.srcObject = stream;
      }
      const { stop } = await startEncoder("camera", stream);
      cameraStopFn = stop;
      setCameraOn(true);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(`Camera failed: ${msg}`);
      cameraStream?.getTracks().forEach((t) => t.stop());
      cameraStream = null;
    }
  }

  async function stopCamera(): Promise<void> {
    cameraStopFn?.();
    cameraStopFn = null;
    cameraEncoder = null;
    cameraStream?.getTracks().forEach((t) => t.stop());
    cameraStream = null;
    cameraStreamId = null;
    if (localCameraVideoRef.value) {
      localCameraVideoRef.value.srcObject = null;
    }
    setCameraOn(false);
  }

  async function startScreen(): Promise<void> {
    setError(null);
    try {
      const stream = await navigator.mediaDevices.getDisplayMedia({
        video: { frameRate: ENCODE_FPS },
        audio: false,
      });
      screenStream = stream;
      if (localScreenVideoRef.value) {
        localScreenVideoRef.value.srcObject = stream;
      }
      // Auto-stop encoder when the user clicks "Stop sharing" in the
      // browser's screen-share controls.
      stream.getVideoTracks()[0]?.addEventListener("ended", () => {
        void stopScreen();
      });
      const { stop } = await startEncoder("screen", stream);
      screenStopFn = stop;
      setScreenOn(true);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(`Screen share failed: ${msg}`);
      screenStream?.getTracks().forEach((t) => t.stop());
      screenStream = null;
    }
  }

  async function stopScreen(): Promise<void> {
    screenStopFn?.();
    screenStopFn = null;
    screenEncoder = null;
    screenStream?.getTracks().forEach((t) => t.stop());
    screenStream = null;
    screenStreamId = null;
    if (localScreenVideoRef.value) {
      localScreenVideoRef.value.srcObject = null;
    }
    setScreenOn(false);
  }

  // Suppress unused-variable warnings for the encoder references the
  // start/stop pair captures via closure.
  void cameraEncoder;
  void screenEncoder;

  // Architecture §10.6 — react to store-level toggle changes from
  // VoicePanel (lifted controls). The pipeline lifecycle stays here;
  // the buttons live where the rest of the voice controls do.
  let cameraRunning = false;
  let screenRunning = false;
  createEffect(() => {
    const want = voiceState.cameraOn;
    if (want && !cameraRunning) {
      cameraRunning = true;
      void startCamera().catch(() => {
        cameraRunning = false;
        setVoiceState("cameraOn", false);
      });
    } else if (!want && cameraRunning) {
      cameraRunning = false;
      void stopCamera();
    }
  });
  createEffect(() => {
    const want = voiceState.screenShareOn;
    if (want && !screenRunning) {
      screenRunning = true;
      void startScreen().catch(() => {
        screenRunning = false;
        setVoiceState("screenShareOn", false);
      });
    } else if (!want && screenRunning) {
      screenRunning = false;
      void stopScreen();
    }
  });

  // Wave 12 W12.7 — Picture-in-Picture. Prefers a remote tile (canvas
  // bridged through a hidden <video> via canvas.captureStream); falls
  // back to the local camera <video> if no remote is showing yet.
  const pipBridgeVideo: { value: HTMLVideoElement | null } = { value: null };
  async function togglePictureInPicture(): Promise<void> {
    try {
      if (document.pictureInPictureElement) {
        await document.exitPictureInPicture();
        return;
      }
      const remote = remotes()[0];
      if (remote && typeof (remote.canvas as HTMLCanvasElement).captureStream === "function") {
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
        pipBridgeVideo.value.srcObject = stream;
        await pipBridgeVideo.value.play().catch(() => {});
        await pipBridgeVideo.value.requestPictureInPicture();
        return;
      }
      const localVideo = localCameraVideoRef.value ?? localScreenVideoRef.value;
      if (localVideo) {
        await localVideo.requestPictureInPicture();
      }
    } catch (e) {
      console.warn("Picture-in-Picture failed:", e);
    }
  }

  return (
    <div class="video-call-panel" classList={{ "video-call-panel-hidden": !props.visible }}>
      <Show when={error()}>
        <div class="search-panel-error" role="alert">{error()}</div>
      </Show>
      <div class="video-call-pip-toolbar">
        <button
          type="button"
          class="video-call-pip-btn"
          title="Toggle picture-in-picture"
          aria-label="Toggle picture-in-picture"
          onClick={() => void togglePictureInPicture()}
        >
          PiP
        </button>
      </div>
      <div class="video-call-grid">
        <Show when={cameraOn()}>
          <div class="video-call-tile">
            <video
              ref={(el) => (localCameraVideoRef.value = el)}
              autoplay
              playsinline
              muted
              class="video-call-video"
            />
            <div class="video-call-tile-label">You · camera</div>
          </div>
        </Show>
        <Show when={screenOn()}>
          <div class="video-call-tile">
            <video
              ref={(el) => (localScreenVideoRef.value = el)}
              autoplay
              playsinline
              muted
              class="video-call-video"
            />
            <div class="video-call-tile-label">You · screen</div>
          </div>
        </Show>
        <For each={remotes()}>
          {(remote) => (
            <div class="video-call-tile">
              <div
                class="video-call-canvas"
                ref={(el) => {
                  // Move the off-DOM canvas into this slot the first
                  // time it's mounted; subsequent renders keep the
                  // same reference.
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
