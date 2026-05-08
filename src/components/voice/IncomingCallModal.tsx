import { Component, Show, createEffect, createSignal, onCleanup } from "solid-js";
import { Dialog } from "@kobalte/core/dialog";
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
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
                  {/* Wave 12 W12.12 — decline-with-reason + temp-mute. */}
                  <DropdownMenu placement="bottom-end">
                    <DropdownMenu.Trigger
                      class="form-btn-secondary incoming-call-decline-more"
                      title="More decline options"
                      aria-label="More decline options"
                    >
                      ▾
                    </DropdownMenu.Trigger>
                    <DropdownMenu.Portal>
                      <DropdownMenu.Content class="context-menu">
                        <DropdownMenu.Item
                          class="context-menu-item"
                          onSelect={() =>
                            void handleDeclineIncomingCall(entry().callId, "I'm busy right now")
                          }
                        >
                          Decline — I'm busy
                        </DropdownMenu.Item>
                        <DropdownMenu.Item
                          class="context-menu-item"
                          onSelect={() =>
                            void handleDeclineIncomingCall(entry().callId, "I'll call back later")
                          }
                        >
                          Decline — I'll call back later
                        </DropdownMenu.Item>
                        <DropdownMenu.Item
                          class="context-menu-item"
                          onSelect={() => {
                            const reason = window.prompt("Decline reason:");
                            if (reason && reason.trim()) {
                              void handleDeclineIncomingCall(entry().callId, reason.trim());
                            }
                          }}
                        >
                          Decline — Custom message…
                        </DropdownMenu.Item>
                        <DropdownMenu.Separator class="context-menu-separator" />
                        <DropdownMenu.Item
                          class="context-menu-item"
                          onSelect={async () => {
                            // 1 hour mute, then decline.
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
                        </DropdownMenu.Item>
                      </DropdownMenu.Content>
                    </DropdownMenu.Portal>
                  </DropdownMenu>
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
