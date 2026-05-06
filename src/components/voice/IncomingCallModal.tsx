import { Component, Show, createEffect, createSignal, onCleanup } from "solid-js";
import { Dialog } from "@kobalte/core/dialog";
import { callsState } from "../../stores/calls.store";
import {
  handleAcceptIncomingCall,
  handleDeclineIncomingCall,
} from "../../handlers/calls.handlers";

/// Plan §Failure 5 — incoming-call notification mounted globally in
/// BuddyListWindow. Reads the head of `callsState.incomingCalls`; the
/// queue advances naturally as the user accepts/declines and the
/// store removes entries.
const IncomingCallModal: Component = () => {
  const head = (): typeof callsState.incomingCalls[number] | undefined =>
    callsState.incomingCalls[0];

  const [now, setNow] = createSignal(Date.now());
  let interval: ReturnType<typeof setInterval> | undefined;

  createEffect(() => {
    if (head()) {
      interval = setInterval(() => setNow(Date.now()), 1000);
      onCleanup(() => {
        if (interval) clearInterval(interval);
        interval = undefined;
      });
    }
  });

  const remainingSeconds = (): number => {
    const entry = head();
    if (!entry) return 0;
    return Math.max(0, Math.ceil((entry.expiresAtMs - now()) / 1000));
  };

  return (
    <Show when={head()}>
      {(entry) => (
        <Dialog open onOpenChange={() => undefined} modal>
          <Dialog.Portal>
            <Dialog.Overlay class="modal-overlay" />
            <div class="modal-overlay-positioner">
              <Dialog.Content class="modal-container modal-container-sm incoming-call-modal">
                <div class="incoming-call-modal-header">
                  <Dialog.Title class="incoming-call-title">
                    Incoming {entry().kind} call
                  </Dialog.Title>
                  <div class="incoming-call-display-name">{entry().displayName}</div>
                  <div class="incoming-call-timer">{remainingSeconds()}s</div>
                </div>
                <div class="incoming-call-actions">
                  <button
                    type="button"
                    class="form-btn-primary incoming-call-accept"
                    onClick={() => void handleAcceptIncomingCall(entry().callId)}
                  >
                    Accept
                  </button>
                  <button
                    type="button"
                    class="form-btn-secondary incoming-call-decline"
                    onClick={() => void handleDeclineIncomingCall(entry().callId)}
                  >
                    Decline
                  </button>
                </div>
              </Dialog.Content>
            </div>
          </Dialog.Portal>
        </Dialog>
      )}
    </Show>
  );
};

export default IncomingCallModal;
