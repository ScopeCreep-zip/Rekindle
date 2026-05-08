import { Component, Show, createEffect, createSignal, onCleanup } from "solid-js";
import { callsState } from "../../stores/calls.store";
import {
  handleAcceptIncomingCall,
  handleDeclineIncomingCall,
  handleEndDmCall,
} from "../../handlers/calls.handlers";

// Wave 12 W12.3 — slim banner that appears when a NEW incoming call
// arrives while an active call is already in progress. The full
// IncomingCallModal would steal focus from the active call's controls,
// so we use Discord/Telegram-style call-waiting UX instead.
//
// Three actions:
//   - "End current and accept"   → endDmCall(active) then accept(incoming)
//   - "Decline"                  → decline(incoming)
//   - "Hold" was considered but skipped — would require protocol-level
//     mute-without-end (no envelope for that yet); revisit if/when a
//     `CallHoldRequest` lands.
const CallWaitingBanner: Component = () => {
  const [now, setNow] = createSignal(Date.now());
  let interval: ReturnType<typeof setInterval> | undefined;

  const incoming = () =>
    callsState.activeCall != null && callsState.incomingCalls.length > 0
      ? callsState.incomingCalls[0]
      : undefined;

  createEffect(() => {
    if (incoming()) {
      interval = setInterval(() => setNow(Date.now()), 1000);
      onCleanup(() => {
        if (interval) clearInterval(interval);
        interval = undefined;
      });
    }
  });

  const remainingSeconds = (): number => {
    const entry = incoming();
    if (!entry) return 0;
    return Math.max(0, Math.ceil((entry.expiresAtMs - now()) / 1000));
  };

  async function endCurrentAndAccept(callId: string): Promise<void> {
    const active = callsState.activeCall;
    if (active) {
      await handleEndDmCall(active.callId, "switching to new call");
    }
    await handleAcceptIncomingCall(callId);
  }

  return (
    <Show when={incoming()}>
      {(entry) => (
        <div class="call-waiting-banner" role="alertdialog" aria-live="polite">
          <div class="call-waiting-banner-text">
            <span class="call-waiting-banner-name">{entry().displayName}</span>
            <span class="call-waiting-banner-kind">
              is calling — {entry().kind} ({remainingSeconds()}s)
            </span>
          </div>
          <div class="call-waiting-banner-actions">
            <button
              type="button"
              class="form-btn-primary"
              onClick={() => void endCurrentAndAccept(entry().callId)}
            >
              End current & answer
            </button>
            <button
              type="button"
              class="form-btn-secondary"
              onClick={() => void handleDeclineIncomingCall(entry().callId)}
            >
              Decline
            </button>
          </div>
        </div>
      )}
    </Show>
  );
};

export default CallWaitingBanner;
