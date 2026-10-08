// Wave 12 W12.1 — Web Audio ringtone / ringback / busy synthesizer.
//
// Synthesized rather than file-based so we don't need to ship/license audio
// assets and there's no remote URL fetch surface for hostile content.
// Three tones with distinguishable cadences:
//   - Incoming   : NA-style dual-tone (440 + 480 Hz), 2 s on / 4 s off
//   - Ringback   : European single-tone (425 Hz),     1 s on / 3 s off
//   - Busy       : NA-style dual-tone (480 + 620 Hz), 0.5 s on / 0.5 s off
//
// Returned RingHandle.stop() fades gain to 0 over 30 ms then disconnects so
// repeated start/stop never produces clicks.

export interface RingHandle {
  stop: () => void;
}

let sharedCtx: AudioContext | null = null;

function audioContext(): AudioContext {
  if (sharedCtx == null) {
    const Ctor =
      window.AudioContext ||
      (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
    sharedCtx = new Ctor();
  }
  // Some browsers suspend the context until first user gesture; resuming is
  // a no-op when already running and silently fails when blocked.
  if (sharedCtx.state === "suspended") {
    void sharedCtx.resume().catch(() => {});
  }
  return sharedCtx;
}

interface ToneSpec {
  frequencies: number[];
  onMs: number;
  offMs: number;
  attackMs: number;
  releaseMs: number;
}

function startTone(spec: ToneSpec, volume: number): RingHandle {
  const ctx = audioContext();
  const clamped = Math.max(0, Math.min(1, volume));

  let stopped = false;
  let cycleTimeout: ReturnType<typeof setTimeout> | undefined;
  let activeOscillators: OscillatorNode[] = [];
  let activeGain: GainNode | null = null;

  const teardownActive = (releaseMs: number): void => {
    const oscs = activeOscillators;
    const gain = activeGain;
    activeOscillators = [];
    activeGain = null;
    if (gain == null) return;
    const now = ctx.currentTime;
    try {
      gain.gain.cancelScheduledValues(now);
      gain.gain.setValueAtTime(gain.gain.value, now);
      gain.gain.linearRampToValueAtTime(0, now + releaseMs / 1000);
    } catch {
      // ignore — context may be closed
    }
    setTimeout(() => {
      for (const osc of oscs) {
        try { osc.stop(); } catch { /* already stopped */ }
        try { osc.disconnect(); } catch { /* ignore */ }
      }
      try { gain.disconnect(); } catch { /* ignore */ }
    }, releaseMs + 5);
  };

  const playOnce = (): void => {
    if (stopped) return;
    const now = ctx.currentTime;
    const gain = ctx.createGain();
    gain.gain.setValueAtTime(0, now);
    gain.gain.linearRampToValueAtTime(clamped, now + spec.attackMs / 1000);
    gain.connect(ctx.destination);
    const oscs: OscillatorNode[] = [];
    for (const freq of spec.frequencies) {
      const osc = ctx.createOscillator();
      osc.type = "sine";
      osc.frequency.setValueAtTime(freq, now);
      osc.connect(gain);
      osc.start(now);
      oscs.push(osc);
    }
    activeOscillators = oscs;
    activeGain = gain;
    cycleTimeout = setTimeout(() => {
      teardownActive(spec.releaseMs);
      cycleTimeout = setTimeout(playOnce, spec.offMs);
    }, spec.onMs);
  };

  playOnce();

  return {
    stop: () => {
      if (stopped) return;
      stopped = true;
      if (cycleTimeout != null) clearTimeout(cycleTimeout);
      teardownActive(30);
    },
  };
}

export function playIncomingRing(opts?: { volume?: number }): RingHandle {
  return startTone(
    {
      frequencies: [440, 480],
      onMs: 2000,
      offMs: 4000,
      attackMs: 10,
      releaseMs: 50,
    },
    opts?.volume ?? 0.4,
  );
}

export function playOutgoingRingback(opts?: { volume?: number }): RingHandle {
  return startTone(
    {
      frequencies: [425],
      onMs: 1000,
      offMs: 3000,
      attackMs: 10,
      releaseMs: 50,
    },
    opts?.volume ?? 0.3,
  );
}

export function playBusyTone(opts?: { volume?: number }): RingHandle {
  return startTone(
    {
      frequencies: [480, 620],
      onMs: 500,
      offMs: 500,
      attackMs: 5,
      releaseMs: 25,
    },
    opts?.volume ?? 0.25,
  );
}
