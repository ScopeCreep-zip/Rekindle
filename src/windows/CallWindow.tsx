import { Component, Show, createMemo, onCleanup, onMount } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import Titlebar from "../components/titlebar/Titlebar";
import ActiveCallPanel from "../components/voice/ActiveCallPanel";
import { produce } from "solid-js/store";
import { callsState, setCallsState } from "../stores/calls.store";
import { reduceActiveCallSnapshot, type CallEffect } from "../stores/calls.reduce";
import { authState } from "../stores/auth.store";
import { addToast } from "../stores/toast.store";
import { errorMessage } from "../utils/error";
import { subscribeCallEvents } from "../handlers/calls.handlers";
import { commands } from "../ipc/commands";
import { setVoiceState } from "../stores/voice.store";
import { startEventStream } from "../ipc/channels";
import { initVoiceEventListener } from "../actions/voice.actions";
import { setWindowKind } from "../utils/window_kind";

/// The call window: the one place a connected 1:1 call's controls and
/// media pipeline run. The backend opens it on connect
/// (`present_active_call`); it reads the call's current state with
/// `get_active_call`, then follows the call's events. When the call ends
/// the backend destroys this window (`retire_call_window`, plan CL2), so
/// its webview and media resources go with it; the "Call ended" fallback
/// shows only when the window opens after its call already ended.
function getCallIdFromUrl(): string {
  const params = new URLSearchParams(window.location.search);
  return params.get("id") ?? "";
}

const CallWindow: Component = () => {
  const callId = getCallIdFromUrl();
  const unlisteners: Promise<UnlistenFn>[] = [];

  /// Load this call's state from the backend: at mount, and again when the
  /// event stream says this window's state may be stale.
  async function load(): Promise<void> {
    const snapshot = await commands.getActiveCall(callId).catch((e: unknown) => {
      addToast(errorMessage(e), "error");
      return null;
    });
    if (!snapshot) return;
    // The snapshot through the store's reducer; its camera policy (a video
    // call starts with the camera on) comes back as an effect.
    let effects: CallEffect[] = [];
    const ctx = { nowMs: Date.now(), selfKey: authState.publicKey ?? "", selfName: authState.displayName ?? "" };
    setCallsState(produce((s) => (effects = reduceActiveCallSnapshot(s, snapshot, ctx))));
    if (effects.some((e) => e.kind === "cameraOn")) setVoiceState("cameraOn", true);
  }

  onMount(async () => {
    unlisteners.push(subscribeCallEvents({ owner: false }));
    // The backend sends a DM call's voice events (mute, speaking,
    // connection quality) to its call window too; without this the
    // panel's peer state never moved (plan C7.28).
    unlisteners.push(initVoiceEventListener());
    void startEventStream(() => void load());
    await load();
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

  setWindowKind("call");
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
