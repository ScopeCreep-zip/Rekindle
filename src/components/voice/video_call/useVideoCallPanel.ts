import { createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { Channel } from "@tauri-apps/api/core";
import { commands } from "../../../ipc/commands";
import type {
  Codec,
  CommunityVideoFrameMsg,
  DmVideoFrameMsg,
  SessionVideoConfig,
} from "../../../ipc/commands";
import { subscribeCommunityEvents } from "../../../ipc/channels";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { setVoiceState, voiceState } from "../../../stores/voice.store";
import { probeAndReportLocalVideoCapabilities } from "../../../handlers/video.handlers";
import { settingsState } from "../../../stores/settings.store";
import {
  setDmPeerDecodeCodecs,
  videoSessionConfigFor,
} from "../../../stores/video.store";
import {
  ACK_INTERVAL_MS,
  DEBUG_VIDEO_LATENCY,
  type RemoteStream,
  decodeBase64ToBytes,
  wireCodecToWebCodecsString,
} from "./codec_utils";
import { VideoPlayoutBuffer } from "./playout_buffer";
import { createVideoSender, type SenderRoute } from "./video_sender";

/** W11.4 — `community` panel routes encoded frames through gossip
 *  fan-out + MEK; `dm` panel routes 1:1 via Signal Double Ratchet. The
 *  decoder side is identical — decoders follow per-frame codec tags. */
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
  /** Phase 11 Tier 1 — high-throughput video frames arrive on a dedicated
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
  // video frames flow over a dedicated per-stream `ipc::Channel`; community
  // control events (acks, keyframe/topology) still ride `community-event`.
  // E2E mode skips Channel registration — channels can't serialize over the
  // HTTP invoke bridge — leaving the rest of the panel inert there.
  onMount(() => {
    const isE2E = import.meta.env.VITE_E2E === "true";
    // Idempotent WebCodecs probe (no-op if handleJoinVoice already ran
    // it). Guarantees the media stack is warm before this panel's
    // first `new VideoDecoder` — a cold WebCodecs call aborts the web
    // process on WebKitGTK 2.52.3 + GStreamer 1.24 (Ubuntu/Pop!_OS
    // 24.04); see handlers/video.handlers.ts.
    if (!isE2E) {
      probeAndReportLocalVideoCapabilities().catch((e) => {
        console.error("WebCodecs capability probe failed:", e);
      });
    }
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
        } else if (event.type === "voicePeerConfirmed") {
          // Three-way handshake leg 3 landed: the joiner is
          // transport-ready. RFC 5104 FIR semantics — a new member
          // needs a full intra to start decoding; force one on every
          // active local stream so their tiles light up immediately
          // instead of waiting out the keyframe cadence.
          if (event.data.channelId === props.channelId) {
            sender.forceKeyframeAll();
          }
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
            msg.codec,
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
          msg.codec,
          msg.timestamp,
          msg.encodedPayloadB64,
        );
      };
      dmFrameChannel = ch;
      void commands.registerDmVideoChannel(props.peerId, ch);
      // Phase 5 — pull the peer's decode codecs (captured from their
      // CallInvite/CallAccept) so the DM sender can intersect against
      // its own encode set. Pre-fetch frames go out as VP9; the
      // sender's per-frame constraints check picks up the store write
      // and reconfigures.
      const peerId = props.peerId;
      void commands
        .dmPeerVideoDecodeCodecs(peerId)
        .then((codecs) => setDmPeerDecodeCodecs(peerId, codecs))
        .catch((e: unknown) => {
          console.error("dmPeerVideoDecodeCodecs fetch failed:", e);
        });
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
    codec: Codec,
    timestamp: number,
    payloadB64: string,
  ): void {
    const data = decodeBase64ToBytes(payloadB64);
    let remote = remotes().find((r) => r.streamId === streamId);
    // Mid-call codec switch (RTP payload-type semantics): a tag change
    // tears down the old decoder; the fresh one seeds from this frame
    // if it's a keyframe, else from the next keyframe.
    if (remote && remote.codec !== codec) {
      try {
        remote.decoder.close();
      } catch (e) {
        console.error("decoder close on codec switch failed:", e);
      }
      setRemotes((prev) => prev.filter((r) => r.streamId !== streamId));
      remote = undefined;
    }
    if (!remote) {
      if (!keyframe) {
        // Wait for the first keyframe before instantiating a decoder.
        return;
      }
      // Phase C — read the negotiated decoder TUNING from the
      // backend-owned store when available (the codec itself comes
      // from the per-frame tag, never the config). When the config
      // hasn't been negotiated yet (late joiner, caps round-trip in
      // flight) we DO NOT drop the keyframe — the old gate here turned
      // that race into a permanently black tile (decoder never
      // created, every later keyframe dropped too). Baseline fallbacks
      // below cover both DM mode and the not-yet-negotiated community
      // case.
      const config =
        props.mode === "community"
          ? videoSessionConfigFor(props.communityId, props.channelId)
          : undefined;
      const decoderOptimizeForLatency =
        config?.decoder.optimizeForLatency ?? false;
      const encoderWidth = config?.encoder.maxWidth ?? 854;
      const encoderHeight = config?.encoder.maxHeight ?? 480;
      // The decoder follows the per-frame codec TAG, never the session
      // config — the config constrains the local ENCODER only.
      const webCodecsString = wireCodecToWebCodecsString(codec);

      const canvas = document.createElement("canvas");
      canvas.width = encoderWidth;
      canvas.height = encoderHeight;
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
          // Architecture §10.6 line 4081 — decoder lost track; ask the
          // sender for a keyframe. Community-only — DM relies on the
          // sender's regular 2-second keyframe cadence. Also surface
          // the failure structurally so the WKWebView / WebKitGTK
          // divergence is observable in backend trace.
          if (props.mode === "community") {
            void commands.reportVideoDecoderStatus(
              props.communityId,
              sender_,
              streamId,
              false,
              e.message,
            );
            void commands.sendVideoKeyframeRequest(props.communityId, props.channelId, streamId);
          }
        },
      });
      const ctx = canvas.getContext("2d");
      remote = {
        streamId,
        senderPseudonym: sender_,
        codec,
        decoder,
        canvas,
        ctx,
        // Flipped true on a successful decoder.configure(); the playout
        // pump skips decode until then.
        ready: false,
        buffer: new VideoPlayoutBuffer(),
        lastAckAt: 0,
        decodeStamps: [],
        lastDecodeMs: 0,
        lastDebugAt: 0,
      };
      // Phase C — configure straight from the negotiated decoder
      // constraints. No probe round-trip: the backend has already
      // negotiated `optimizeForLatency` against every peer's
      // capability report, so a synchronous configure here either
      // succeeds (set ready=true) or fails (report + leave ready=false).
      const created = remote;
      try {
        decoder.configure({
          codec: webCodecsString,
          optimizeForLatency: decoderOptimizeForLatency,
        });
        created.ready = true;
        if (props.mode === "community") {
          void commands.reportVideoDecoderStatus(
            props.communityId,
            sender_,
            streamId,
            true,
          );
        }
      } catch (e) {
        const errorMessage = e instanceof Error ? e.message : String(e);
        if (props.mode === "community") {
          void commands.reportVideoDecoderStatus(
            props.communityId,
            sender_,
            streamId,
            false,
            errorMessage,
          );
        }
      }
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

  /** Capture / display constraints come from the backend-negotiated
   *  encoder config (Phase B / C). DM mode (no community context) uses
   *  the baseline 480p@15 floor — the codec pick is the sender's
   *  concern (see video_sender.ts pickDmEncoderCodec). */
  function captureConstraints(): { width: number; height: number; frameRate: number } {
    if (props.mode === "community") {
      const config = videoSessionConfigFor(props.communityId, props.channelId);
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

  async function startCamera(): Promise<void> {
    setError(null);
    try {
      // Plan §Failure 2 — honour the persisted camera selection from
      // Settings → Video. `exact` surfaces an OverconstrainedError if the
      // device disappeared so the user sees a clear message.
      const deviceId = settingsState.selectedVideoDeviceId;
      const { width, height, frameRate } = captureConstraints();
      const stream = await navigator.mediaDevices.getUserMedia({
        video: {
          deviceId: deviceId ? { exact: deviceId } : undefined,
          width,
          height,
          frameRate,
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
      const { frameRate } = captureConstraints();
      const stream = await navigator.mediaDevices.getDisplayMedia({
        video: { frameRate },
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

  // Phase C — when the backend re-negotiates the session config (a
  // weaker / stronger peer joined or left), tear down every remote
  // decoder so the next inbound keyframe rebuilds it against the new
  // constraints. Sender-side encoder teardown lives in video_sender.ts
  // — the sender subscribes to the same store. DM mode has no
  // community config, so the effect short-circuits there.
  //
  // The first emit after a fresh subscription is part of the steady
  // state (every effect runs once on creation). `previousConfigRef`
  // skips that first run so we don't tear down a remote that never
  // existed.
  let previousConfig: SessionVideoConfig | undefined;
  createEffect(() => {
    if (props.mode !== "community") return;
    const config = videoSessionConfigFor(props.communityId, props.channelId);
    if (config === undefined) return;
    if (previousConfig === undefined) {
      previousConfig = config;
      return;
    }
    if (
      previousConfig.encoder.codec === config.encoder.codec &&
      previousConfig.encoder.maxWidth === config.encoder.maxWidth &&
      previousConfig.encoder.maxHeight === config.encoder.maxHeight &&
      previousConfig.encoder.maxFps === config.encoder.maxFps &&
      previousConfig.encoder.scalabilityMode === config.encoder.scalabilityMode &&
      previousConfig.decoder.optimizeForLatency === config.decoder.optimizeForLatency
    ) {
      return;
    }
    previousConfig = config;
    // Drop every remote — the pump skips closed decoders and the next
    // inbound keyframe re-seeds a fresh RemoteStream with the new
    // decoder constraints from the store.
    setRemotes((prev) => {
      for (const r of prev) {
        try {
          r.decoder.close();
        } catch (e) {
          console.error("decoder close on config change failed:", e);
        }
      }
      return [];
    });
  });

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
