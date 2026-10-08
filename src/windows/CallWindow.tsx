import { Component, Show, createMemo, onCleanup, onMount } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import Titlebar from "../components/titlebar/Titlebar";
import ActiveCallPanel from "../components/voice/ActiveCallPanel";
import { callsState, setCallsState, type CallEntry } from "../stores/calls.store";
import { subscribeCallEvents } from "../handlers/calls.handlers";
import { commands } from "../ipc/commands";
import { setVoiceState } from "../stores/voice.store";
import { startEventStream } from "../ipc/channels";

/// The call window: the one place a connected 1:1 call's controls and
/// media pipeline run. The backend opens it on connect
/// (`present_active_call`); it reads the call's current state with
/// `get_active_call`, then follows the call's events.
function getCallIdFromUrl(): string {
  const params = new URLSearchParams(window.location.search);
  return params.get("id") ?? "";
}

const CallWindow: Component = () => {
  const callId = getCallIdFromUrl();
  const unlisteners: Promise<UnlistenFn>[] = [];

  onMount(async () => {
    unlisteners.push(subscribeCallEvents({ owner: false }));
    void startEventStream();
    const snapshot = await commands.getActiveCall(callId);
    if (!snapshot) return;
    const entry: CallEntry = {
      callId: snapshot.callId,
      peerKey: snapshot.peerKey,
      displayName: snapshot.displayName,
      kind: snapshot.kind,
      expiresAtMs: snapshot.expiresAtMs,
      startedAtMs: Date.now(),
    };
    setCallsState(snapshot.connected ? "activeCall" : "outgoingCall", entry);
    // The window opens right after `connected`, which carries the camera
    // policy, so it applies the same policy from the snapshot: a video
    // call starts with the camera on.
    if (snapshot.connected && snapshot.kind === "video") {
      setVoiceState("cameraOn", true);
    }
  });

  onCleanup(() => {
    for (const p of unlisteners) p.then((u) => u());
  });

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
