import { Component, Show, createMemo } from "solid-js";
import Titlebar from "../components/titlebar/Titlebar";
import ActiveCallPanel from "../components/voice/ActiveCallPanel";
import { callsState } from "../stores/calls.store";

// Wave 12 W12.7 — pop-out call window. Mounts the same ActiveCallPanel
// as the inline DmWindow surface but in `popout` mode (full-window).
//
// State sync between the originating webview and this popout: each
// Tauri webview is its own JS context, so they each maintain their own
// `callsState`. The CallController in main.tsx subscribes to the same
// chat-event channel in EVERY webview, so both windows see the
// `callConnected` / `callMediaStateChanged` / `callEnded` events
// independently and stay in sync.
//
// On unmount (or when the user closes the popout window) the call
// remains active — the inline panel in the originating window picks
// back up. On hangup from this window, the backend emits CallEnded
// globally and both windows tear down.
function getCallIdFromUrl(): string {
  const params = new URLSearchParams(window.location.search);
  return params.get("id") ?? "";
}

const CallWindow: Component = () => {
  const callId = getCallIdFromUrl();

  const call = createMemo(() => {
    const a = callsState.activeCall;
    if (a && a.callId === callId) return a;
    const o = callsState.outgoingCall;
    if (o && o.callId === callId) return o;
    return null;
  });

  return (
    <div class="app-frame call-window-frame">
      <Titlebar title="Call" />
      <Show
        when={call()}
        fallback={
          <div class="empty-placeholder">
            <div class="empty-placeholder-title">Call ended</div>
            <div class="empty-placeholder-subtitle">
              You can close this window.
            </div>
          </div>
        }
      >
        {(c) => <ActiveCallPanel call={c()} mode="popout" />}
      </Show>
    </div>
  );
};

export default CallWindow;
