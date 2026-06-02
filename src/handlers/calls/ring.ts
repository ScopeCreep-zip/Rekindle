import type { RingHandle } from "../../utils/ringtone";

// Wave 12 W12.1 — single in-flight ring handle per webview. Replaced
// when a new ring would start (e.g. caller declined while another offer
// already arrived); cleared on every terminal call event.
let activeRing: RingHandle | null = null;

export function stopActiveRing(): void {
  if (activeRing != null) {
    activeRing.stop();
    activeRing = null;
  }
}

/// Stop any in-flight ring and start a new one. Every ring transition in
/// the call-event handlers stops the prior handle before assigning, so
/// this captures that stop-then-set invariant in one place.
export function startRing(handle: RingHandle): void {
  stopActiveRing();
  activeRing = handle;
}
