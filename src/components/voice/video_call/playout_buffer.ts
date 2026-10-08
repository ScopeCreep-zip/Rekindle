// Architecture §10.6 — receiver-side video jitter/playout buffer. Mirrors
// crates/rekindle-voice/src/jitter.rs (which keeps voice smooth): reorder by
// sequence, fill an initial cushion, release on a paced clock, drop late, jump
// to a keyframe across a gap. Without this, frames painted the instant they
// arrive turn gossip-mesh inter-arrival jitter directly into visible
// choppiness. Pure: no DOM, no Tauri — the only stateful video logic the thin
// frontend owns, kept isolated and reviewable.
//
// VP9 delta frames reference priors, so frames MUST decode in sequence order.
// This buffer therefore reorders the *encoded* chunks and paces their *release
// into the decoder*; the decoder's output callback still paints immediately.
// Single stage, single clock — the videocall-codecs / libwebrtc model.
// delay = jitterEstimate × multiplier, clamped to [min, max]. These are
// *call* bounds (live), not *streaming* bounds: WebRTC's video jitter
// buffer lives at ~50–200ms and libwebrtc's low-latency start delay is
// ~30–40ms. The 500ms/×3 streaming defaults pinned delay near the
// ceiling on bursty gossip arrival and put video ~1s behind live.
export const PLAYOUT_MIN_DELAY_MS = 40; // libwebrtc low-latency start delay
export const PLAYOUT_MAX_DELAY_MS = 180; // WebRTC video jitter-buffer ceiling
export const PLAYOUT_JITTER_MULTIPLIER = 2.5;
export const PLAYOUT_MAX_FRAMES = 30; // ~2s @15fps — a runaway guard, not a buffer

export interface BufferedFrame {
  frameSeq: number;
  keyframe: boolean;
  /** Sender ms (performance.now); becomes EncodedVideoChunk.timestamp. */
  timestamp: number;
  data: Uint8Array;
  /** Local performance.now() at ingest — drives jitter + due time. */
  receivedAt: number;
}

/** Result of one playout pump: chunks to decode now + a gap signal. */
export interface PlayoutBatch {
  /** In-order, decodable chunks ready to hand the VideoDecoder now. */
  release: BufferedFrame[];
  /** A decode gap forced a keyframe jump (or stall) — ask the sender. */
  requestKeyframe: boolean;
}

export class VideoPlayoutBuffer {
  private frames = new Map<number, BufferedFrame>();
  private nextSeq: number | null = null; // next seq to release, in order
  private playoutDelayMs = PLAYOUT_MIN_DELAY_MS;
  private jitterMs = 0;
  private lastArrival: number | null = null;
  private lastTimestamp: number | null = null;
  private initialFillDone = false;
  private firstFrameAt: number | null = null;

  push(frame: BufferedFrame): void {
    // RFC3550 inter-arrival jitter EWMA → adaptive playout delay.
    if (this.lastArrival !== null && this.lastTimestamp !== null) {
      const transit = frame.receivedAt - frame.timestamp;
      const lastTransit = this.lastArrival - this.lastTimestamp;
      const d = Math.abs(transit - lastTransit);
      this.jitterMs += (d - this.jitterMs) / 16;
    }
    this.lastArrival = frame.receivedAt;
    this.lastTimestamp = frame.timestamp;
    this.playoutDelayMs = Math.min(
      PLAYOUT_MAX_DELAY_MS,
      Math.max(PLAYOUT_MIN_DELAY_MS, this.jitterMs * PLAYOUT_JITTER_MULTIPLIER),
    );

    // Late-drop: already played past this seq, or a duplicate.
    if (this.nextSeq !== null && frame.frameSeq < this.nextSeq) return;
    if (this.frames.has(frame.frameSeq)) return;

    this.frames.set(frame.frameSeq, frame);
    if (this.firstFrameAt === null) this.firstFrameAt = frame.receivedAt;
    // Can only start a stream on a keyframe (deltas before it are undecodable).
    if (this.nextSeq === null && frame.keyframe) this.nextSeq = frame.frameSeq;

    // Hard cap — evict the smallest seq if over budget.
    while (this.frames.size > PLAYOUT_MAX_FRAMES) {
      const min = this.minSeq();
      if (min === null) break;
      this.frames.delete(min);
      if (this.nextSeq !== null && min >= this.nextSeq) this.nextSeq = min + 1;
    }
  }

  /** Pull every chunk due for decode at `now`, in seq order. */
  popDue(now: number): PlayoutBatch {
    const out: PlayoutBatch = { release: [], requestKeyframe: false };
    if (this.nextSeq === null || this.firstFrameAt === null) return out;

    // Initial fill: hold until one playout-delay of frames has buffered.
    if (!this.initialFillDone) {
      if (now - this.firstFrameAt < this.playoutDelayMs) return out;
      this.initialFillDone = true;
    }

    for (;;) {
      const next = this.frames.get(this.nextSeq);
      if (next) {
        // In order, but only once aged past the playout delay so an
        // out-of-order prior has had its window to arrive.
        if (now - next.receivedAt < this.playoutDelayMs) break;
        out.release.push(next);
        this.frames.delete(this.nextSeq);
        this.nextSeq += 1;
        continue;
      }
      // Hole at nextSeq. Only treat it as loss once newer frames have aged
      // out the delay — otherwise the missing one may still be in flight.
      const min = this.minSeq();
      if (min === null || min <= this.nextSeq) break;
      const oldest = this.frames.get(min)!;
      if (now - oldest.receivedAt < this.playoutDelayMs) break;
      // A hole at [nextSeq, min) whose successors have aged out: the
      // reference chain is broken here, so advance past it and force a
      // keyframe (below). This gap is a DECODE signal only — wire loss
      // is measured in the Rust receive path over the transport
      // sequence, never over these frameSeq gaps (which also open when
      // the SENDER's pacer expires frames — not loss at all).
      this.nextSeq = min;
      const kf = this.nextKeyframeSeq();
      if (kf === null) {
        // Only undecodable deltas ahead — they reference the now-broken
        // chain and can never decode. Drop them (delivered-but-undecodable,
        // already counted as received, NOT wire loss), hold here, and ask
        // for a fresh keyframe. Clearing the buffer also makes the next
        // stalled tick a no-op so the gap isn't re-counted.
        this.frames.clear();
        out.requestKeyframe = true;
        break;
      }
      // A keyframe is buffered ahead: drop the undecodable deltas before it
      // ([min, kf) — delivered-but-undecodable, not counted) and resume from
      // the keyframe.
      for (const k of [...this.frames.keys()]) if (k < kf) this.frames.delete(k);
      this.nextSeq = kf;
      out.requestKeyframe = true;
    }
    return out;
  }

  /** Read-only snapshot for the render-path latency log (DEBUG_VIDEO_LATENCY).
   *  Isolates the buffer's own contribution (delay + occupancy) from the
   *  decoder's internal latency measured separately at the decode→output edge. */
  debugStats(): { size: number; playoutDelayMs: number; jitterMs: number; nextSeq: number | null } {
    return {
      size: this.frames.size,
      playoutDelayMs: Math.round(this.playoutDelayMs),
      jitterMs: Math.round(this.jitterMs),
      nextSeq: this.nextSeq,
    };
  }

  private minSeq(): number | null {
    let min: number | null = null;
    for (const k of this.frames.keys()) if (min === null || k < min) min = k;
    return min;
  }

  private nextKeyframeSeq(): number | null {
    const from = this.nextSeq ?? 0;
    let best: number | null = null;
    for (const f of this.frames.values()) {
      if (f.keyframe && f.frameSeq >= from && (best === null || f.frameSeq < best)) {
        best = f.frameSeq;
      }
    }
    return best;
  }
}
