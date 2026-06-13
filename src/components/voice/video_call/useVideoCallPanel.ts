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
import {
  setDmPeerDecodeCodecs,
  videoSessionConfigFor,
} from "../../../stores/video.store";
import {
  ACK_INTERVAL_MS,
  DEBUG_VIDEO_LATENCY,
  KEYFRAME_REQUEST_MIN_INTERVAL_MS,
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
   *  mount, never OS-sniffed. When a native session runs, its loopback
   *  stream id marks the self-view stream in ingest. */
  let nativeCaptureAvailable = false;
  let nativeStreamId: string | null = null;
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
      playoutHandle = window.setTimeout(playoutPump, 250);
    } else {
      playoutVia = "raf";
      playoutHandle = requestAnimationFrame(playoutPump);
    }
  }

  /** Re-arm across the visibility edge — a pending rAF in a newly
   *  hidden window may never fire, killing the chain before it could
   *  reschedule onto the timer path. */
  function onPlayoutVisibilityChange(): void {
    cancelPlayout();
    schedulePlayout();
  }

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
      commands
        .nativeVideoCaptureAvailable()
        .then((available) => {
          nativeCaptureAvailable = available;
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
            if (nativeStreamId) {
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
          if (event.data.channelId === props.channelId && !nativeStreamId) {
            sender.setTargetKbps(event.data.kbps);
          }
        } else if (event.type === "nativeVideoError") {
          // The backend camera session died asynchronously (unplug,
          // pipeline failure) — revert the toggle and surface it.
          if (event.data.channelId === props.channelId) {
            nativeStreamId = null;
            setCameraOn(false);
            setError(`Camera stopped: ${event.data.message}`);
          }
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

  // Per-stream count of delta frames dropped while waiting for a
  // keyframe (decoder not yet created). Cleared when the keyframe
  // arrives; drives the explicit keyframe-request escalation.
  const keyframeWaitDrops = new Map<string, number>();

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
      console.warn(
        `codec switch ${remote.codec} → ${codec} on stream ${streamId.slice(0, 8)} — decoder torn down`,
      );
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
        // Waiting for the first keyframe before instantiating a
        // decoder. If we landed mid-GOP (joined while the sender was
        // between keyframes, or the FIR-on-confirm envelope was lost),
        // deltas pile up here — after 15 of them, explicitly request a
        // keyframe so the tile lights up within ~1s instead of waiting
        // out the sender's keyframe cadence.
        const dropped = (keyframeWaitDrops.get(streamId) ?? 0) + 1;
        keyframeWaitDrops.set(streamId, dropped);
        if (dropped === 1 || dropped % 30 === 0) {
          console.warn(
            `dropping delta frames for unknown stream ${streamId.slice(0, 8)} — waiting for keyframe (${dropped} dropped)`,
          );
        }
        // Every 15th dropped delta, not a one-shot at 15: the request
        // envelope is fire-and-forget, so a single lost request used to
        // freeze the tile until the sender's own keyframe cadence.
        if (dropped % 15 === 0) {
          requestKeyframeFor(streamId);
        }
        return;
      }
      keyframeWaitDrops.delete(streamId);
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
      const ctx = canvas.getContext("2d");
      remote = {
        streamId,
        senderPseudonym: sender_,
        isLocal: nativeStreamId !== null && streamId === nativeStreamId,
        codec,
        // Placeholder — installDecoder() below replaces it before the
        // remote is appended; never decoded against.
        decoder: undefined as unknown as VideoDecoder,
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
        lastDecoderRebuildAt: 0,
        awaitKeyframe: false,
      };
      installDecoder(remote, webCodecsString, decoderOptimizeForLatency);
      setRemotes((prev) => [...prev, remote!]);
    }

    // Self-view loopback: frames arrive in order over the ipc::Channel
    // — there is nothing to reorder or absorb, and the playout buffer
    // would add ~50 ms of pure self-view lag. Decode immediately
    // (keeping the fresh-decoder keyframe guard).
    if (remote.isLocal) {
      if (!remote.ready) return;
      if (remote.awaitKeyframe) {
        if (!keyframe) {
          requestKeyframeFor(remote.streamId);
          return;
        }
        remote.awaitKeyframe = false;
      }
      // R5 latency probe: the native pump stamps unix-ms (u32-wrapped)
      // timestamps, so encode→ingest is directly measurable. ~1 Hz
      // through the *-settings info lane — the webview console is
      // invisible in dev logs.
      const nowMs = performance.now();
      if (nowMs - remote.lastDebugAt >= 1000) {
        remote.lastDebugAt = nowMs;
        const e2e = (Date.now() % 4294967295) - timestamp;
        void commands.reportMediaCaptureError(
          "self-view-settings",
          `loopback encode→ingest ${e2e}ms (decode+paint adds single-digit ms)`,
        );
      }
      try {
        remote.decoder.decode(
          new EncodedVideoChunk({
            type: keyframe ? "key" : "delta",
            timestamp,
            data,
          }),
        );
      } catch (e) {
        console.error("self-view decode failed:", e);
        requestKeyframeFor(remote.streamId);
      }
      return;
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

  /** Create + configure a WebCodecs decoder onto `r`. Called at stream
   *  creation AND from `recoverDecoder` — a fatal WebCodecs decoder
   *  error CLOSES the decoder permanently (field: one undecryptable
   *  frame killed the remote feed for the whole session while 71
   *  keyframe requests went to a corpse). The error callback therefore
   *  rebuilds instead of only requesting a keyframe. */
  function installDecoder(
    r: RemoteStream,
    webCodecsString: string,
    optimizeForLatency: boolean,
  ): void {
    const decoder = new VideoDecoder({
      output: (frame: VideoFrame) => {
        const target = remotes().find((t) => t.streamId === r.streamId);
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
        if (props.mode === "community") {
          void commands.reportVideoDecoderStatus(
            props.communityId,
            r.senderPseudonym,
            r.streamId,
            false,
            e.message,
          );
        }
        recoverDecoder(r.streamId, webCodecsString, optimizeForLatency);
      },
    });
    r.decoder = decoder;
    try {
      decoder.configure({
        codec: webCodecsString,
        optimizeForLatency,
      });
      r.ready = true;
      if (props.mode === "community") {
        void commands.reportVideoDecoderStatus(
          props.communityId,
          r.senderPseudonym,
          r.streamId,
          true,
        );
      }
    } catch (e) {
      const errorMessage = e instanceof Error ? e.message : String(e);
      r.ready = false;
      if (props.mode === "community") {
        void commands.reportVideoDecoderStatus(
          props.communityId,
          r.senderPseudonym,
          r.streamId,
          false,
          errorMessage,
        );
      }
    }
  }

  /** Rebuild a fatally-errored decoder (closed state is permanent in
   *  WebCodecs), cooldown-guarded against error-loop thrash. The fresh
   *  decoder must see a keyframe first — `awaitKeyframe` makes the
   *  pump skip deltas until one decodes — and the sender is asked for
   *  one immediately. */
  const DECODER_REBUILD_COOLDOWN_MS = 3000;
  function recoverDecoder(
    streamId: string,
    webCodecsString: string,
    optimizeForLatency: boolean,
  ): void {
    const r = remotes().find((t) => t.streamId === streamId);
    if (!r) return;
    const now = performance.now();
    if (now - r.lastDecoderRebuildAt < DECODER_REBUILD_COOLDOWN_MS) return;
    r.lastDecoderRebuildAt = now;
    r.ready = false;
    try {
      r.decoder.close();
    } catch {
      // Already closed by the fatal error — expected.
    }
    r.decodeStamps.length = 0;
    r.awaitKeyframe = true;
    installDecoder(r, webCodecsString, optimizeForLatency);
    requestKeyframeFor(streamId);
  }

  /** Community-only: ask the sender to emit a keyframe so a decoder that lost
   *  track (gap / decode error) can re-sync. DM relies on the periodic cadence.
   *  Rate-limited per stream (1 Hz) — callers may invoke every pump tick while
   *  desynced; persistence beats reliability over a fire-and-forget envelope. */
  const keyframeRequestAt = new Map<string, number>();
  function requestKeyframeFor(streamId: string): void {
    if (props.mode !== "community") return;
    const now = performance.now();
    const last = keyframeRequestAt.get(streamId) ?? 0;
    if (now - last < KEYFRAME_REQUEST_MIN_INTERVAL_MS) return;
    keyframeRequestAt.set(streamId, now);
    // The self-view loopback's sender is OUR native encoder — channel
    // envelopes would spam peers with requests for a stream they don't
    // own while the frozen tile never heard them.
    if (nativeStreamId !== null && streamId === nativeStreamId) {
      void commands.forceNativeKeyframes();
      return;
    }
    void commands.sendVideoKeyframeRequest(props.communityId, props.channelId, streamId);
  }

  /** Drives every remote's playout buffer: release due chunks in order,
   *  decode them, recover gaps via keyframe request, and ack measured
   *  kbps/loss (~1 Hz) so the sender's adaptive bitrate has real input. */
  function playoutPump(): void {
    const now = performance.now();
    for (const r of remotes()) {
      // Skip until the async decoder.configure() has landed (see ingest).
      if (!r.ready) continue;
      // Self-view loopback streams decode at ingest (in-order channel,
      // nothing to jitter-absorb) and are NEVER acked — a self-ack
      // would broadcast loopback stats into every peer's channel-keyed
      // bitrate policy.
      if (r.isLocal) continue;
      const { release, requestKeyframe } = r.buffer.popDue(now);
      for (const f of release) {
        // A rebuilt decoder must see a keyframe before any delta —
        // feeding it one is itself a fatal error (rebuild loop).
        if (r.awaitKeyframe) {
          if (!f.keyframe) {
            requestKeyframeFor(r.streamId);
            continue;
          }
          r.awaitKeyframe = false;
        }
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
    schedulePlayout();
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

  async function startCamera(): Promise<void> {
    setError(null);
    // Backend-native capture path (capability-detected): no
    // getUserMedia, no webview encoder — the backend owns the camera
    // and encode, and the self view arrives via the loopback stream.
    // Re-query on a cold cache: a click racing the onMount probe must
    // not fall back to the webview encoder on a native-capable box
    // (the probe is OnceLock-cached backend-side — this is cheap).
    if (!nativeCaptureAvailable && props.mode === "community") {
      nativeCaptureAvailable = await commands
        .nativeVideoCaptureAvailable()
        .catch(() => false);
    }
    if (nativeCaptureAvailable && props.mode === "community") {
      try {
        const prefs = await commands.getPreferences();
        nativeStreamId = await commands.startNativeVideo(
          props.communityId,
          props.channelId,
          "camera",
          prefs.videoDeviceLabel ?? null,
        );
        setCameraOn(true);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        nativeStreamId = null;
        setError(`Camera failed: ${msg}`);
        void commands.reportMediaCaptureError("camera-native", msg);
      }
      return;
    }
    const { width, height, frameRate } = captureConstraints();
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
        setError("Saved camera unavailable — using default camera");
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
      cameraStream = stream;
      setCameraCapture(stream);
      if (localCameraVideoRef.value) {
        localCameraVideoRef.value.srcObject = stream;
      }
      // Capture + local preview only — the egress effect attaches the
      // sender when (and only when) the media-ready gate is open, so a
      // solo member sees their own tile immediately and sending starts
      // the moment a peer connects.
      setCameraOn(true);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(`Camera failed: ${msg}`);
      void commands.reportMediaCaptureError("camera", msg);
      cameraStream?.getTracks().forEach((t) => t.stop());
      cameraStream = null;
      setCameraCapture(null);
    }
  }

  async function stopCamera(): Promise<void> {
    if (nativeStreamId !== null) {
      const localId = nativeStreamId;
      nativeStreamId = null;
      // Drop the loopback tile + decoder with the session.
      setRemotes((prev) =>
        prev.filter((r) => {
          if (r.streamId === localId) {
            try {
              r.decoder.close();
            } catch (e) {
              console.error("self-view decoder close failed:", e);
            }
            return false;
          }
          return true;
        }),
      );
      try {
        await commands.stopNativeVideo();
      } catch (e) {
        console.error("stop_native_video failed:", e);
      }
      setCameraOn(false);
      return;
    }
    sender.stop("camera");
    cameraStream?.getTracks().forEach((t) => t.stop());
    cameraStream = null;
    setCameraCapture(null);
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
      setScreenCapture(stream);
      if (localScreenVideoRef.value) {
        localScreenVideoRef.value.srcObject = stream;
      }
      // Auto-stop encoder when the user clicks "Stop sharing" in the
      // browser's screen-share controls.
      stream.getVideoTracks()[0]?.addEventListener("ended", () => {
        void stopScreen();
      });
      // Capture + preview only — sender attaches via the egress effect.
      setScreenOn(true);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(`Screen share failed: ${msg}`);
      void commands.reportMediaCaptureError("screen", msg);
      screenStream?.getTracks().forEach((t) => t.stop());
      screenStream = null;
      setScreenCapture(null);
    }
  }

  async function stopScreen(): Promise<void> {
    sender.stop("screen");
    screenStream?.getTracks().forEach((t) => t.stop());
    screenStream = null;
    setScreenCapture(null);
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
      // PiP shows a REMOTE peer — the self-view loopback must not win
      // the slot just because it registered first.
      const remote = remotes().find((r) => !r.isLocal);
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
