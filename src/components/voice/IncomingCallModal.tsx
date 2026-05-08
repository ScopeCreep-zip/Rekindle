import { Component, Show, createEffect, createSignal, onCleanup } from "solid-js";
import { Dialog } from "@kobalte/core/dialog";
import { callsState } from "../../stores/calls.store";
import {
  handleAcceptIncomingCall,
  handleDeclineIncomingCall,
} from "../../handlers/calls.handlers";
import { commands } from "../../ipc/commands";

/// Wave 12 W12.1+W12.3 — incoming-call modal mounted globally in
/// CallController. Reads the head of `callsState.incomingCalls`. While
/// no call is active we use the full overlay; when an active call is in
/// progress, CallWaitingBanner takes over so the modal doesn't steal
/// the active call's controls. The queue advances naturally as the
/// user accepts / declines and the store removes entries.
const IncomingCallModal: Component = () => {
  const head = (): typeof callsState.incomingCalls[number] | undefined => {
    if (callsState.activeCall != null) return undefined;
    return callsState.incomingCalls[0];
  };

  const [now, setNow] = createSignal(Date.now());
  // W13-fix.3 — inline expand-on-click for the decline-with-reason
  // menu. Replaces a Kobalte DropdownMenu nested inside the modal
  // Dialog whose Portal collided with the Dialog overlay's z-index,
  // making clicks on the menu items invisible to the user. Inline
  // section = no portal conflict.
  const [showDeclineOptions, setShowDeclineOptions] = createSignal(false);
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
                  {/* W13-fix.3 — inline toggle (was nested DropdownMenu
                   *  whose Portal collided with the Dialog overlay's
                   *  z-index, making clicks invisible). */}
                  <button
                    type="button"
                    class="form-btn-secondary incoming-call-decline-more"
                    title="More decline options"
                    aria-label="More decline options"
                    aria-expanded={showDeclineOptions()}
                    onClick={() => setShowDeclineOptions((v) => !v)}
                  >
                    {showDeclineOptions() ? "▴" : "▾"}
                  </button>
                </div>
                <Show when={showDeclineOptions()}>
                  <div class="incoming-call-decline-options" role="group" aria-label="Decline with reason">
                    <button
                      type="button"
                      class="form-btn-secondary"
                      onClick={() =>
                        void handleDeclineIncomingCall(entry().callId, "I'm busy right now")
                      }
                    >
                      I'm busy
                    </button>
                    <button
                      type="button"
                      class="form-btn-secondary"
                      onClick={() =>
                        void handleDeclineIncomingCall(entry().callId, "I'll call back later")
                      }
                    >
                      Call back later
                    </button>
                    <button
                      type="button"
                      class="form-btn-secondary"
                      onClick={() => {
                        const reason = window.prompt("Decline reason:");
                        if (reason && reason.trim()) {
                          void handleDeclineIncomingCall(entry().callId, reason.trim());
                        }
                      }}
                    >
                      Custom message…
                    </button>
                    <button
                      type="button"
                      class="form-btn-secondary"
                      onClick={async () => {
                        await commands
                          .muteCallerTemp(entry().peerKey, 60 * 60 * 1000)
                          .catch((e) => console.warn("muteCallerTemp:", e));
                        void handleDeclineIncomingCall(
                          entry().callId,
                          "user is unavailable",
                        );
                      }}
                    >
                      Mute caller for 1 hour
                    </button>
                  </div>
                </Show>
              </Dialog.Content>
            </div>
          </Dialog.Portal>
        </Dialog>
      )}
    </Show>
  );
};

export default IncomingCallModal;
