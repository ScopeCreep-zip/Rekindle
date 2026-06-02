import { createEffect, createMemo, createSignal, onCleanup } from "solid-js";
import type { MessageInputProps } from "../MessageInput";

// Persists the most recent successful-send timestamp per channel/peer across
// MessageInput remounts (switching channels destroys the component but the map
// is module-scoped so the cooldown survives).
const lastSendAtMs = new Map<string, number>();

// Architecture §slowmode — per-channel send cooldown. The component is told
// whether sending is disabled (`effectiveDisabled`) and why
// (`effectiveDisabledMessage`); `recordSent` stamps the last successful send.
export function useSlowmode(props: MessageInputProps) {
  const [now, setNow] = createSignal(Date.now());

  // Tick once per second so the countdown updates. Only when slowmode applies.
  createEffect(() => {
    const seconds = props.slowmodeSeconds ?? 0;
    if (seconds <= 0 || props.bypassSlowmode) return;
    const interval = window.setInterval(() => setNow(Date.now()), 1000);
    onCleanup(() => window.clearInterval(interval));
  });

  const cooldownRemainingMs = createMemo(() => {
    const seconds = props.slowmodeSeconds ?? 0;
    if (seconds <= 0 || props.bypassSlowmode) return 0;
    const last = lastSendAtMs.get(props.peerId) ?? 0;
    const cooldown = seconds * 1000;
    const elapsed = now() - last;
    return Math.max(0, cooldown - elapsed);
  });

  const cooldownActive = createMemo(() => cooldownRemainingMs() > 0);
  const cooldownLabel = createMemo(() => {
    const remaining = cooldownRemainingMs();
    if (remaining <= 0) return "";
    const seconds = Math.ceil(remaining / 1000);
    return `Slowmode: wait ${seconds}s before sending`;
  });

  const effectiveDisabled = createMemo(() =>
    Boolean(props.disabled) || (cooldownActive() && !props.editMode),
  );
  const effectiveDisabledMessage = createMemo(() => {
    if (cooldownActive() && !props.editMode) return cooldownLabel();
    return props.disabledMessage ?? "You cannot send messages here";
  });

  function recordSent(): void {
    if ((props.slowmodeSeconds ?? 0) > 0 && !props.bypassSlowmode) {
      lastSendAtMs.set(props.peerId, Date.now());
      setNow(Date.now());
    }
  }

  return { effectiveDisabled, effectiveDisabledMessage, recordSent };
}
