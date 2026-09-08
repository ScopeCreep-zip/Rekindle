import { createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { Channel } from "@tauri-apps/api/core";
import { commands } from "../../../ipc/commands";
import type {
  CommunityVideoFrameMsg,
  DmVideoFrameMsg,
  SessionVideoConfig,
} from "../../../ipc/commands";
import { subscribeCommunityEvents } from "../../../ipc/channels";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { setVoiceState, voiceState } from "../../../stores/voice.store";
import { probeAndReportLocalVideoCapabilities } from "../../../actions/video.actions";
import { setDmPeerDecodeCodecs, videoSessionConfigFor } from "../../../stores/video.store";
import type { RemoteStream } from "./codec_utils";
import { createCaptureController } from "./panel_capture";
import type { PanelCtx, Ref } from "./panel_ctx";
import { createDecodePipeline } from "./panel_decode";
import { createPipToggle } from "./panel_pip";
import { createVideoSender, type SenderRoute } from "./video_sender";

// Declared in ./panel_ctx.ts and re-exported here for VideoCallPanel.tsx.
// `PanelCtx` names these props, and this hook builds the ctx from the
// modules that consume it (panel_capture / panel_decode / panel_pip),
// so declaring the type here closed four module cycles.
export type { VideoCallPanelProps } from "./panel_ctx";
import type { VideoCallPanelProps } from "./panel_ctx";

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

  // Reactive mirrors of the capture streams — the EGRESS effects below
  // attach/detach the sender from these, so local preview (capture)
  // and network send are independent lifecycles.
  const [cameraCapture, setCameraCapture] = createSignal<MediaStream | null>(null);
  const [screenCapture, setScreenCapture] = createSignal<MediaStream | null>(null);

  let unlistenCommunity: Promise<UnlistenFn> | null = null;
  /** Phase 11 Tier 1 — high-throughput video frames arrive on a dedicated
   *  per-stream `ipc::Channel` (registered with the backend on mount),
   *  not the shared event bus. DM mode keys by peer pubkey; community mode
   *  by community id. Control events still ride `community-event`. */
  let dmFrameChannel: Channel<DmVideoFrameMsg> | null = null;
  let communityFrameChannel: Channel<CommunityVideoFrameMsg> | null = null;
  /** Backend-native capture (Linux GStreamer) — capability-detected on
   *  mount, never OS-sniffed. `ctx.nativeStreamId` non-null marks an
   *  active native session: encode is GStreamer's (VP9 to peers), the
   *  self-view is a direct getUserMedia preview, and keyframe FIR /
   *  bitrate events route to the native encoder instead of the webview
   *  one. Reactive so the call stage can decide the self-tile rendering
   *  (canvas vs <video>) from a STABLE per-session flag. */
  const [nativeCaptureAvailable, setNativeCaptureAvailable] = createSignal(false);
  /** This session captures natively (Linux GStreamer) — community mode
   *  only; DM stays on the webview path. Stable for the session. */
  const nativeCapture = (): boolean => nativeCaptureAvailable() && props.mode === "community";
  /** Linux-native self-view: the backend capture pipeline tees a JPEG
   *  preview branch to us (single capture, no 2nd getUserMedia consumer,
   *  no loopback). Those stills paint to this canvas, which the
   *  self-camera tile mounts. Null on webview platforms (mac/Windows use
   *  a direct getUserMedia `<video>` instead). */
  const [nativeSelfCanvas, setNativeSelfCanvas] = createSignal<HTMLCanvasElement | null>(null);

  /** Single clock that paces every remote's playout buffer into its
   *  decoder — and emits the ~1 Hz acks the sender's bitrate policy
   *  feeds on. rAF when visible; a timer chain when hidden, because
   *  every platform's webview suspends rAF for hidden/minimized
   *  windows (Page Visibility semantics) — a frozen pump stalls
   *  decode AND acks, which the far end can't tell apart from a dead
   *  route. */
  let playoutHandle: number | null = null;
  let playoutVia: "raf" | "timer" = "raf";

  function cancelPlayout(): void {
    if (playoutHandle === null) return;
    if (playoutVia === "raf") cancelAnimationFrame(playoutHandle);
    else clearTimeout(playoutHandle);
    playoutHandle = null;
  }

  function schedulePlayout(): void {
    if (document.hidden) {
      playoutVia = "timer";
      playoutHandle = window.setTimeout(() => decode.playoutPump(), 250);
    } else {
      playoutVia = "raf";
      playoutHandle = requestAnimationFrame(() => decode.playoutPump());
    }
  }

  /** Re-arm across the visibility edge — a pending rAF in a newly
   *  hidden window may never fire, killing the chain before it could
   *  reschedule onto the timer path. */
  function onPlayoutVisibilityChange(): void {
    cancelPlayout();
    schedulePlayout();
  }

  const localCameraVideoRef: Ref<HTMLVideoElement | undefined> = { value: undefined };
  const localScreenVideoRef: Ref<HTMLVideoElement | undefined> = { value: undefined };

  // The extracted pipeline modules (panel_decode / panel_capture /
  // panel_pip) are plain functions of this shared context — signals stay
  // hook-created; effects and lifecycle registration never leave the
  // hook (SolidJS owner scope).
  const ctx: PanelCtx = {
    props,
    sender,
    remotes,
    setRemotes,
    setError,
    setCameraOn,
    setScreenOn,
    setCameraCapture,
    setScreenCapture,
    nativeCaptureAvailable,
    setNativeCaptureAvailable,
    setNativeSelfCanvas,
    cameraStream: null,
    screenStream: null,
    nativeStreamId: null,
    nativePreviewChannel: null,
    nativePreviewDecoding: false,
    localCameraVideoRef,
    localScreenVideoRef,
    schedulePlayout,
  };
  const decode = createDecodePipeline(ctx);
  const capture = createCaptureController(ctx);
  const togglePictureInPicture = createPipToggle(ctx);

  // The pipeline now lives in a headless host (CallPipelineHost) so the
  // call survives navigating between channels. The local-preview <video>
  // elements, however, are mounted/unmounted by the call-stage gallery.
  // These binders let a freshly-mounted gallery tile reattach the live
  // MediaStream (the broadcast captures from the stream directly — see
  // video_sender.ts — so the preview element is purely cosmetic).
  function bindCameraVideo(el: HTMLVideoElement | null): void {
    localCameraVideoRef.value = el ?? undefined;
    if (el) el.srcObject = ctx.cameraStream;
  }
  function bindScreenVideo(el: HTMLVideoElement | null): void {
    localScreenVideoRef.value = el ?? undefined;
    if (el) el.srcObject = ctx.screenStream;
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
      commands
        .nativeVideoCaptureAvailable()
        .then((available) => {
          setNativeCaptureAvailable(available);
        })
        .catch(() => {
          // Probe failure = webview path; same as off-Linux.
        });
    }
    // One clock paces playout for all remotes. E2E has no decoders/frames,
    // so the loop is harmless there (remotes() stays empty).
    schedulePlayout();
    document.addEventListener("visibilitychange", onPlayoutVisibilityChange);
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
            if (ctx.nativeStreamId) {
              void commands.forceNativeKeyframes();
            } else {
              sender.forceKeyframeAll();
            }
          }
        } else if (event.type === "videoBitrateTarget") {
          // Phase 4 — the BACKEND owns the bitrate policy now (AIMD
          // over FrameAck/BandwidthEstimate feedback with the audio
          // reserve subtracted). The encoder just follows the target;
          // the raw ack events no longer steer it directly. The native
          // path follows the bitrate watch in the backend already.
          if (event.data.channelId === props.channelId && !ctx.nativeStreamId) {
            sender.setTargetKbps(event.data.kbps);
          }
        } else if (event.type === "nativeVideoError") {
          // The backend camera session died asynchronously (unplug,
          // pipeline failure) — revert the toggle and surface it.
          if (event.data.channelId === props.channelId) {
            ctx.nativeStreamId = null;
            void commands.unregisterNativePreviewChannel();
            ctx.nativePreviewChannel = null;
            setNativeSelfCanvas(null);
            setCameraOn(false);
            setError(`Camera stopped: ${event.data.message}`);
          }
        }
      });
      if (!isE2E) {
        const ch = new Channel<CommunityVideoFrameMsg>();
        ch.onmessage = (msg) => {
          decode.ingestRemoteFrame(
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
        decode.ingestRemoteFrame(
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
    void capture.stopCamera();
    void capture.stopScreen();
    document.removeEventListener("visibilitychange", onPlayoutVisibilityChange);
    cancelPlayout();
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

  // Phase C — when the backend re-negotiates the session config (a
  // weaker / stronger peer joined or left), tear down every remote
  // decoder so the next inbound keyframe rebuilds it against the new
  // constraints. Sender-side encoder teardown lives in video_sender.ts
  // — the sender subscribes to the same store. DM mode has no
  // community config, so the effect short-circuits there.
  //
  // The first emit after a fresh subscription is part of the steady
  // state (every effect runs once on creation). `previousConfig`
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
  //
  // CAPTURE (getUserMedia + local preview) follows the toggle alone —
  // a solo member sees their own tile immediately, exactly like every
  // comparable app. EGRESS (encoder + network send) is a separate
  // effect gated on the backend media-ready state, so frames are never
  // produced for an empty roster and sending starts automatically the
  // moment the gate opens (peer joined, handshake converged, MEK in).
  let cameraRunning = false;
  let screenRunning = false;
  const mediaGateOpen = (): boolean =>
    props.mode !== "community" || (voiceState.mediaReady?.ready ?? false);
  createEffect(() => {
    const want = voiceState.cameraOn;
    if (want && !cameraRunning) {
      cameraRunning = true;
      void capture.startCamera().catch(() => {
        cameraRunning = false;
        setVoiceState("cameraOn", false);
      });
    } else if (!want && cameraRunning) {
      cameraRunning = false;
      void capture.stopCamera();
    }
  });
  createEffect(() => {
    const want = voiceState.screenShareOn;
    if (want && !screenRunning) {
      screenRunning = true;
      void capture.startScreen().catch(() => {
        screenRunning = false;
        setVoiceState("screenShareOn", false);
      });
    } else if (!want && screenRunning) {
      screenRunning = false;
      void capture.stopScreen();
    }
  });
  // EGRESS effects — attach/detach the sender pipeline. Detaching on
  // gate close (channel emptied) keeps the preview alive while the
  // encoder stops feeding the backend gate.
  let cameraSending = false;
  createEffect(() => {
    const stream = cameraCapture();
    const shouldSend = voiceState.cameraOn && stream !== null && mediaGateOpen();
    if (shouldSend && !cameraSending) {
      cameraSending = true;
      void sender.start("camera", stream).catch((e) => {
        cameraSending = false;
        const msg = e instanceof Error ? e.message : String(e);
        setError(`Camera send failed: ${msg}`);
      });
    } else if (!shouldSend && cameraSending) {
      cameraSending = false;
      sender.stop("camera");
    }
  });
  let screenSending = false;
  createEffect(() => {
    const stream = screenCapture();
    const shouldSend = voiceState.screenShareOn && stream !== null && mediaGateOpen();
    if (shouldSend && !screenSending) {
      screenSending = true;
      void sender.start("screen", stream).catch((e) => {
        screenSending = false;
        const msg = e instanceof Error ? e.message : String(e);
        setError(`Screen share send failed: ${msg}`);
      });
    } else if (!shouldSend && screenSending) {
      screenSending = false;
      sender.stop("screen");
    }
  });

  return {
    error,
    remotes,
    cameraOn,
    screenOn,
    localCameraVideoRef,
    localScreenVideoRef,
    bindCameraVideo,
    bindScreenVideo,
    /** This session captures natively (Linux): the self-tile is the
     *  JPEG preview canvas, never a getUserMedia `<video>`. Stable per
     *  session, so the call stage never races a half-built tile. */
    nativeCapture,
    /** Linux-native self-view canvas (JPEG preview branch); null until
     *  the first preview frame creates it. */
    nativeSelfCanvas,
    togglePictureInPicture,
  };
}

export type VideoCallVm = ReturnType<typeof useVideoCallPanel>;
