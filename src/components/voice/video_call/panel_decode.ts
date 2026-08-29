// Receiver pipeline — frame ingest, decoder lifecycle (install/recover),
// keyframe-request escalation, and the playout pump. Extracted from
// useVideoCallPanel.ts; plain functions of the shared PanelCtx (no
// SolidJS owner-scoped primitives here).

import { commands } from "../../../ipc/commands";
import type { Codec } from "../../../ipc/commands";
import { videoSessionConfigFor } from "../../../stores/video.store";
import {
  ACK_INTERVAL_MS,
  DEBUG_VIDEO_LATENCY,
  KEYFRAME_REQUEST_MIN_INTERVAL_MS,
  type RemoteStream,
  decodeBase64ToBytes,
  wireCodecToWebCodecsString,
} from "./codec_utils";
import type { PanelCtx } from "./panel_ctx";
import { VideoPlayoutBuffer } from "./playout_buffer";

/** Rebuild cooldown for a fatally-errored decoder (closed state is
 *  permanent in WebCodecs) — guards against error-loop thrash. */
const DECODER_REBUILD_COOLDOWN_MS = 3000;

export interface DecodePipeline {
  ingestRemoteFrame(
    sender: string,
    streamId: string,
    frameSeq: number,
    keyframe: boolean,
    codec: Codec,
    timestamp: number,
    payloadB64: string,
  ): void;
  /** Community-only 1 Hz-limited keyframe request for a desynced stream. */
  requestKeyframeFor(streamId: string): void;
  /** Drives every remote's playout buffer; re-arms via ctx.schedulePlayout. */
  playoutPump(): void;
}

export function createDecodePipeline(ctx: PanelCtx): DecodePipeline {
  // Per-stream count of delta frames dropped while waiting for a
  // keyframe (decoder not yet created). Cleared when the keyframe
  // arrives; drives the explicit keyframe-request escalation.
  const keyframeWaitDrops = new Map<string, number>();

  /** Community-only: ask the sender to emit a keyframe so a decoder that lost
   *  track (gap / decode error) can re-sync. DM relies on the periodic cadence.
   *  Rate-limited per stream (1 Hz) — callers may invoke every pump tick while
   *  desynced; persistence beats reliability over a fire-and-forget envelope. */
  const keyframeRequestAt = new Map<string, number>();
  function requestKeyframeFor(streamId: string): void {
    if (ctx.props.mode !== "community") return;
    const now = performance.now();
    const last = keyframeRequestAt.get(streamId) ?? 0;
    if (now - last < KEYFRAME_REQUEST_MIN_INTERVAL_MS) return;
    keyframeRequestAt.set(streamId, now);
    void commands.sendVideoKeyframeRequest(
      ctx.props.communityId,
      ctx.props.channelId,
      streamId,
    );
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
        const target = ctx.remotes().find((t) => t.streamId === r.streamId);
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
        if (ctx.props.mode === "community") {
          void commands.reportVideoDecoderStatus(
            ctx.props.communityId,
            r.senderPseudonym,
            r.streamId,
            false,
            `${e.message} [codec=${webCodecsString}]`,
          );
        }
        recoverDecoder(r.streamId, webCodecsString, optimizeForLatency);
      },
    });
    r.decoder = decoder;
    try {
      // optimizeForLatency is deliberately NOT set: WebKitGTK's
      // WebCodecs low-latency decode path is buggy (Igalia: decoder
      // not pinned to a single thread) and produces "Decode error" on
      // otherwise-valid VP8/VP9 frames — the configure succeeds but the
      // first decode throws, looping the decoder rebuild. Correctness
      // over the ~1 frame of latency the hint would save. (`optimize
      // ForLatency` is still threaded through for the error report and
      // the recover path so the diagnostic stays honest.)
      decoder.configure({
        codec: webCodecsString,
      });
      r.ready = true;
      if (ctx.props.mode === "community") {
        void commands.reportVideoDecoderStatus(
          ctx.props.communityId,
          r.senderPseudonym,
          r.streamId,
          true,
        );
      }
    } catch (e) {
      const errorMessage = e instanceof Error ? e.message : String(e);
      r.ready = false;
      if (ctx.props.mode === "community") {
        void commands.reportVideoDecoderStatus(
          ctx.props.communityId,
          r.senderPseudonym,
          r.streamId,
          false,
          errorMessage,
        );
      }
    }
  }

  /** Rebuild a fatally-errored decoder, cooldown-guarded against
   *  error-loop thrash. The fresh decoder must see a keyframe first —
   *  `awaitKeyframe` makes the pump skip deltas until one decodes —
   *  and the sender is asked for one immediately. */
  function recoverDecoder(
    streamId: string,
    webCodecsString: string,
    optimizeForLatency: boolean,
  ): void {
    const r = ctx.remotes().find((t) => t.streamId === streamId);
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
    let remote = ctx.remotes().find((r) => r.streamId === streamId);
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
      ctx.setRemotes((prev) => prev.filter((r) => r.streamId !== streamId));
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
        ctx.props.mode === "community"
          ? videoSessionConfigFor(ctx.props.communityId, ctx.props.channelId)
          : undefined;
      const decoderOptimizeForLatency = config?.decoder.optimizeForLatency ?? false;
      const encoderWidth = config?.encoder.maxWidth ?? 854;
      const encoderHeight = config?.encoder.maxHeight ?? 480;
      // The decoder follows the per-frame codec TAG, never the session
      // config — the config constrains the local ENCODER only.
      const webCodecsString = wireCodecToWebCodecsString(codec);

      const canvas = document.createElement("canvas");
      canvas.width = encoderWidth;
      canvas.height = encoderHeight;
      const ctx2d = canvas.getContext("2d");
      remote = {
        streamId,
        senderPseudonym: sender_,
        codec,
        // Placeholder — installDecoder() below replaces it before the
        // remote is appended; never decoded against.
        decoder: undefined as unknown as VideoDecoder,
        canvas,
        ctx: ctx2d,
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
      ctx.setRemotes((prev) => [...prev, remote!]);
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

  /** Drives every remote's playout buffer: release due chunks in order,
   *  decode them, recover gaps via keyframe request, and ack measured
   *  kbps/loss (~1 Hz) so the sender's adaptive bitrate has real input. */
  function playoutPump(): void {
    const now = performance.now();
    for (const r of ctx.remotes()) {
      // Skip until the async decoder.configure() has landed (see ingest).
      if (!r.ready) continue;
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
      if (ctx.props.mode === "community" && now - r.lastAckAt >= ACK_INTERVAL_MS) {
        r.lastAckAt = now;
        const { kbps, lossQ8, lastFrameSeq } = r.buffer.takeStats(now);
        void commands.sendVideoFrameAck(
          ctx.props.communityId,
          ctx.props.channelId,
          r.streamId,
          lastFrameSeq,
          kbps,
          lossQ8,
        );
      }
    }
    ctx.schedulePlayout();
  }

  return { ingestRemoteFrame, requestKeyframeFor, playoutPump };
}
