import { Component, Show, createEffect, createSignal, onCleanup } from "solid-js";
import { callsState } from "../../stores/calls.store";
import { handleEndDmCall } from "../../handlers/calls.handlers";

// Wave 12 W12.4 — outgoing-call panel: "Calling X…" + countdown + Cancel.
// The synthesized ringback tone is started/stopped by the calls handler
// (handleStartDmCall / terminal call events) so the panel itself stays
// purely visual.
//
// Mounted globally inside CallController so the panel surfaces no
// matter which window initiated the call (e.g., the user can start a
// call from the BuddyListWindow context menu and the "Calling Bob…"
// panel appears in their current window).
const OutgoingCallPanel: Component = () => {
  const [now, setNow] = createSignal(Date.now());
  let interval: ReturnType<typeof setInterval> | undefined;

  const outgoing = (): typeof callsState.outgoingCall => callsState.outgoingCall;

  createEffect(() => {
    if (outgoing()) {
      interval = setInterval(() => setNow(Date.now()), 1000);
      onCleanup(() => {
        if (interval) clearInterval(interval);
        interval = undefined;
      });
    }
  });

  const remainingSeconds = (): number => {
    const entry = outgoing();
    if (!entry) return 0;
    return Math.max(0, Math.ceil((entry.expiresAtMs - now()) / 1000));
  };

  return (
    <Show when={outgoing()}>
      {(entry) => (
        <div class="outgoing-call-panel" role="alertdialog" aria-live="polite">
          <div class="outgoing-call-panel-name">{entry().displayName}</div>
          <div class="outgoing-call-panel-kind">
            {entry().status === "ringing" ? "Ringing…" : "Calling…"} {entry().kind} call
          </div>
          <div class="outgoing-call-panel-timer">{remainingSeconds()}s</div>
          <button
            type="button"
            class="form-btn-secondary outgoing-call-panel-cancel"
            disabled={!entry().callId}
            title={!entry().callId ? "Waiting for the network…" : "Cancel call"}
            onClick={() => {
              const id = entry().callId;
              if (id) void handleEndDmCall(id, "cancelled by caller");
            }}
          >
            Cancel
          </button>
        </div>
      )}
    </Show>
  );
};

export default OutgoingCallPanel;
