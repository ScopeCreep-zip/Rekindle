import { createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { Channel } from "@tauri-apps/api/core";
import { commands } from "../../../ipc/commands";
import type { CommunityVideoFrameMsg, DmVideoFrameMsg } from "../../../ipc/commands";
import { subscribeCommunityEvents } from "../../../ipc/channels";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { setVoiceState, voiceState } from "../../../stores/voice.store";
import { settingsState } from "../../../stores/settings.store";
import {
  ENCODE_WIDTH,
  ENCODE_HEIGHT,
  ENCODE_FPS,
  ACK_INTERVAL_MS,
  DEBUG_VIDEO_LATENCY,
  type RemoteStream,
  decodeBase64ToBytes,
} from "./codec_utils";
import { VideoPlayoutBuffer } from "./playout_buffer";
import { createVideoSender, type SenderRoute } from "./video_sender";

/** W11.4 — `community` panel routes encoded frames through gossip
 *  fan-out + MEK; `dm` panel routes 1:1 via Signal Double Ratchet. The
 *  decoder side is identical (WebCodecs VP9). */
export type VideoCallPanelProps =
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

const DECODER_CODEC = "vp09.00.30.08";

// `optimizeForLatency` tells the decoder to minimise frames buffered before
// output — but it's a hint not all engines honour (WKWebView / WebView2 /
// WebKitGTK-via-GStreamer differ), so probe it through isConfigSupported and
// fall back to the plain config. Probed once per process and cached; later
// decoders resolve the settled promise on a microtask.
let optimizeForLatencyProbe: Promise<boolean> | null = null;
function probeOptimizeForLatency(): Promise<boolean> {
  if (!optimizeForLatencyProbe) {
    optimizeForLatencyProbe = VideoDecoder.isConfigSupported({
      codec: DECODER_CODEC,
      optimizeForLatency: true,
    })
      .then((r) => r.supported === true)
      .catch(() => false);
  }
  return optimizeForLatencyProbe;
}

export function useVideoCallPanel(props: VideoCallPanelProps) {
  // Architecture §10.6 — desired state lives in the voice store so the
  // toggle buttons in VoicePanel stay visible regardless of whether
  // VideoCallPanel is mounted. Local accessors mirror the store.
  const cameraOn = (): boolean => voiceState.cameraOn;
  const setCameraOn = (next: boolean): void => setVoiceState("cameraOn", next);
  const screenOn = (): boolean => voiceState.screenShareOn;
  const setScreenOn = (next: boolean): void => setVoiceState("screenShareOn", next);
  const [error, setError] = createSignal<string | null>(null);
  const [remotes, setRemotes] = createSignal<RemoteStream[]>([]);

  const route: SenderRoute =
    props.mode === "community"
      ? { mode: "community", communityId: props.communityId, channelId: props.channelId }
      : { mode: "dm", peerId: props.peerId };
  const sender = createVideoSender(route, setError);

  let cameraStream: MediaStream | null = null;
  let screenStream: MediaStream | null = null;

  let unlistenCommunity: Promise<UnlistenFn> | null = null;
  /** Phase 11 Tier 1 — high-throughput VP9 frames arrive on a dedicated
   *  per-stream `ipc::Channel` (registered with the backend on mount),
   *  not the shared event bus. DM mode keys by peer pubkey; community mode
   *  by community id. Control events still ride `community-event`. */
  let dmFrameChannel: Channel<DmVideoFrameMsg> | null = null;
  let communityFrameChannel: Channel<CommunityVideoFrameMsg> | null = null;
  /** Single rAF that paces every remote's playout buffer into its decoder. */
  let playoutRaf: number | null = null;

  const localCameraVideoRef: { value: HTMLVideoElement | undefined } = { value: undefined };
  const localScreenVideoRef: { value: HTMLVideoElement | undefined } = { value: undefined };

  // The pipeline now lives in a headless host (CallPipelineHost) so the
  // call survives navigating between channels. The local-preview <video>
  // elements, however, are mounted/unmounted by the call-stage gallery.
  // These binders let a freshly-mounted gallery tile reattach the live
  // MediaStream (the broadcast captures from the stream directly — see
  // video_sender.ts — so the preview element is purely cosmetic).
  function bindCameraVideo(el: HTMLVideoElement | null): void {
    localCameraVideoRef.value = el ?? undefined;
    if (el) el.srcObject = cameraStream;
  }
  function bindScreenVideo(el: HTMLVideoElement | null): void {
    localScreenVideoRef.value = el ?? undefined;
    if (el) el.srcObject = screenStream;
  }

  // Architecture §10.6 / Phase 11 Tier 1 — receiver pipeline. Reassembled
  // VP9 frames flow over a dedicated per-stream `ipc::Channel`; community
  // control events (acks, keyframe/topology) still ride `community-event`.
  // E2E mode skips Channel registration — channels can't serialize over the
  // HTTP invoke bridge — leaving the rest of the panel inert there.
  onMount(() => {
    const isE2E = import.meta.env.VITE_E2E === "true";
    // One rAF clock paces playout for all remotes. E2E has no decoders/frames,
    // so the loop is harmless there (remotes() stays empty).
    playoutRaf = requestAnimationFrame(playoutPump);
    if (props.mode === "community") {
      const communityIdLocal = props.communityId;
      unlistenCommunity = subscribeCommunityEvents((event) => {
        if (event.type === "videoTopologyChange") {
          const { communityId, streamId } = event.data;
          if (communityId !== communityIdLocal) return;
          // Drop matching remote so the next frame seeds a fresh decoder.
          setRemotes((prev) =>
            prev.filter((r) => {
              if (r.streamId === streamId) {
                try {
                  r.decoder.close();
                } catch (e) {
                  console.error("decoder close failed:", e);
                }
                return false;
              }
              return true;
            }),
          );
        } else if (event.type === "videoKeyframeRequest") {
          sender.forceKeyframe(event.data.streamId);
        } else if (event.type === "videoFrameAck") {
          // Architecture §10.6 line 4081 — adapt encoder bitrate to the
          // slowest receiver. configure() picks up the new value on the
          // next keyframe interval.
          sender.noteReceiverKbps(event.data.streamId, event.data.kbps);
        } else if (event.type === "videoBandwidthEstimate") {
          // Out-of-band, channel-scoped hint; clamp both streams.
          sender.noteBandwidth(event.data.kbps);
        }
      });
      if (!isE2E) {
        const ch = new Channel<CommunityVideoFrameMsg>();
        ch.onmessage = (msg) => {
          ingestRemoteFrame(
            msg.senderPseudonym,
            msg.streamId,
            msg.frameSeq,
            msg.keyframe,
            msg.timestamp,
            msg.payloadB64,
          );
        };
        communityFrameChannel = ch;
        void commands.registerCommunityVideoChannel(communityIdLocal, ch);
      }
    } else if (!isE2E) {
      const ch = new Channel<DmVideoFrameMsg>();
      ch.onmessage = (msg) => {
        ingestRemoteFrame(
          msg.peerPubkey,
          msg.streamIdHex,
          msg.frameSeq,
          msg.keyframe,
          msg.timestamp,
          msg.encodedPayloadB64,
        );
      };
      dmFrameChannel = ch;
      void commands.registerDmVideoChannel(props.peerId, ch);
    }
  });

  onCleanup(() => {
    void stopCamera();
    void stopScreen();
    if (playoutRaf !== null) cancelAnimationFrame(playoutRaf);
    unlistenCommunity?.then((unlisten) => unlisten());
    if (communityFrameChannel && props.mode === "community") {
      void commands.unregisterCommunityVideoChannel(props.communityId);
    }
    if (dmFrameChannel && props.mode === "dm") {
      void commands.unregisterDmVideoChannel(props.peerId);
    }
    for (const r of remotes()) {
      try {
        r.decoder.close();
      } catch (e) {
        console.error("decoder close failed:", e);
      }
    }
  });

  function ingestRemoteFrame(
    sender_: string,
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
          if (DEBUG_VIDEO_LATENCY) {
            // Pair this output with its decode() call to measure the decoder's
            // internal latency (decode→paint), isolated from buffer delay.
            const t0 = target.decodeStamps.shift();
            if (t0 !== undefined) target.lastDecodeMs = performance.now() - t0;
          }
          target.ctx.drawImage(frame, 0, 0, target.canvas.width, target.canvas.height);
          frame.close();
        },
        error: (e: Error) => {
          console.error("VideoDecoder error:", e);
          // Architecture §10.6 line 4081 — decoder lost track; ask the
          // sender for a keyframe. Community-only — DM relies on the
          // sender's regular 2-second keyframe cadence.
          if (props.mode === "community") {
            void commands.sendVideoKeyframeRequest(props.communityId, props.channelId, streamId);
          }
        },
      });
      const ctx = canvas.getContext("2d");
      remote = {
        streamId,
        senderPseudonym: sender_,
        decoder,
        canvas,
        ctx,
        // Flipped true once the async probe + configure below completes; the
        // playout pump skips decode until then.
        ready: false,
        buffer: new VideoPlayoutBuffer(),
        lastAckAt: 0,
        decodeStamps: [],
        lastDecodeMs: 0,
        lastDebugAt: 0,
      };
      // Configure off the cached latency-hint probe, then open the gate. The
      // first keyframe waits in the buffer (initial-fill delay) so the pump
      // won't try to decode before this lands. Probing here — not reconfiguring
      // a live decoder — avoids a key-chunk-required hitch mid-stream.
      const created = remote;
      void probeOptimizeForLatency().then((supported) => {
        if (decoder.state === "closed") return;
        try {
          decoder.configure(
            supported
              ? { codec: DECODER_CODEC, optimizeForLatency: true }
              : { codec: DECODER_CODEC },
          );
          created.ready = true;
        } catch (e) {
          console.error("decoder configure failed:", e);
        }
      });
      setRemotes((prev) => [...prev, remote!]);
    }

    // Reorder + jitter-absorb instead of decoding on arrival. The playout
    // pump (started in onMount) releases due chunks in seq order and paints
    // them on a steady clock — the fix for choppiness.
    remote.buffer.push({
      frameSeq,
      keyframe,
      timestamp,
      data,
      receivedAt: performance.now(),
    });
  }

  /** Community-only: ask the sender to emit a keyframe so a decoder that lost
   *  track (gap / decode error) can re-sync. DM relies on the periodic cadence. */
  function requestKeyframeFor(streamId: string): void {
    if (props.mode === "community") {
      void commands.sendVideoKeyframeRequest(props.communityId, props.channelId, streamId);
    }
  }

  /** Drives every remote's playout buffer: release due chunks in order,
   *  decode them, recover gaps via keyframe request, and ack measured
   *  kbps/loss (~1 Hz) so the sender's adaptive bitrate has real input. */
  function playoutPump(): void {
    const now = performance.now();
    for (const r of remotes()) {
      // Skip until the async decoder.configure() has landed (see ingest).
      if (!r.ready) continue;
      const { release, requestKeyframe } = r.buffer.popDue(now);
      for (const f of release) {
        try {
          if (DEBUG_VIDEO_LATENCY) r.decodeStamps.push(performance.now());
          r.decoder.decode(
            new EncodedVideoChunk({
              type: f.keyframe ? "key" : "delta",
              timestamp: f.timestamp,
              data: f.data,
            }),
          );
        } catch (e) {
          // decode() threw — no output callback will fire, so drop the stamp
          // we just pushed to keep the FIFO aligned with real outputs.
          if (DEBUG_VIDEO_LATENCY) r.decodeStamps.pop();
          console.error("decode chunk failed:", e);
          requestKeyframeFor(r.streamId);
        }
      }
      if (requestKeyframe) requestKeyframeFor(r.streamId);
      if (DEBUG_VIDEO_LATENCY && now - r.lastDebugAt >= 1000) {
        r.lastDebugAt = now;
        const s = r.buffer.debugStats();
        console.debug(
          `[video ${r.streamId.slice(0, 8)}] playoutDelay=${s.playoutDelayMs}ms ` +
            `bufSize=${s.size} decodeQueue=${r.decoder.decodeQueueSize} ` +
            `lastDecode=${Math.round(r.lastDecodeMs)}ms jitter=${s.jitterMs}ms`,
        );
      }
      if (props.mode === "community" && now - r.lastAckAt >= ACK_INTERVAL_MS) {
        r.lastAckAt = now;
        const { kbps, lossQ8, lastFrameSeq } = r.buffer.takeStats(now);
        void commands.sendVideoFrameAck(
          props.communityId,
          props.channelId,
          r.streamId,
          lastFrameSeq,
          kbps,
          lossQ8,
        );
      }
    }
    playoutRaf = requestAnimationFrame(playoutPump);
  }

  async function startCamera(): Promise<void> {
    setError(null);
    try {
      // Plan §Failure 2 — honour the persisted camera selection from
      // Settings → Video. `exact` surfaces an OverconstrainedError if the
      // device disappeared so the user sees a clear message.
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
      await sender.start("camera", stream);
      setCameraOn(true);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(`Camera failed: ${msg}`);
      cameraStream?.getTracks().forEach((t) => t.stop());
      cameraStream = null;
    }
  }

  async function stopCamera(): Promise<void> {
    sender.stop("camera");
    cameraStream?.getTracks().forEach((t) => t.stop());
    cameraStream = null;
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
      await sender.start("screen", stream);
      setScreenOn(true);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(`Screen share failed: ${msg}`);
      screenStream?.getTracks().forEach((t) => t.stop());
      screenStream = null;
    }
  }

  async function stopScreen(): Promise<void> {
    sender.stop("screen");
    screenStream?.getTracks().forEach((t) => t.stop());
    screenStream = null;
    if (localScreenVideoRef.value) {
      localScreenVideoRef.value.srcObject = null;
    }
    setScreenOn(false);
  }

  // Architecture §10.6 — react to store-level toggle changes from
  // VoicePanel (lifted controls). The pipeline lifecycle stays here; the
  // buttons live where the rest of the voice controls do.
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

  return {
    error,
    remotes,
    cameraOn,
    screenOn,
    localCameraVideoRef,
    localScreenVideoRef,
    bindCameraVideo,
    bindScreenVideo,
    togglePictureInPicture,
  };
}

export type VideoCallVm = ReturnType<typeof useVideoCallPanel>;
