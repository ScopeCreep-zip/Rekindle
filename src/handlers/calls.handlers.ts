import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeChatEvents } from "../ipc/channels";
import { callsState, setCallsState, type CallEntry } from "../stores/calls.store";
import { commands } from "../ipc/commands";
import { addToast } from "../stores/toast.store";

/// Plan §Failure 5 — wires the four `chat-event` variants the backend
/// emits during a direct call into the `calls.store`. Components read
/// from the store; nothing else owns call state.
export function subscribeCallEvents(): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    switch (event.type) {
      case "incomingCall": {
        const { callId, from, displayName, kind, expiresAtMs } = event.data;
        const entry: CallEntry = {
          callId,
          peerKey: from,
          displayName,
          kind,
          expiresAtMs,
          startedAtMs: Date.now(),
        };
        // Queue if multiple offers race in — IncomingCallModal shows
        // the head of the list.
        setCallsState("incomingCalls", (prev) => [...prev, entry]);
        break;
      }
      case "callConnected": {
        const { callId } = event.data;
        // Promote either the outgoing call (Alice path) or the head
        // incoming call (Bob path) to the active slot.
        const out = callsState.outgoingCall;
        if (out && out.callId === callId) {
          setCallsState("activeCall", out);
          setCallsState("outgoingCall", null);
          break;
        }
        const idx = callsState.incomingCalls.findIndex((c) => c.callId === callId);
        if (idx >= 0) {
          const entry = callsState.incomingCalls[idx];
          setCallsState("activeCall", entry);
          setCallsState("incomingCalls", (prev) => prev.filter((_, i) => i !== idx));
        }
        break;
      }
      case "callTimedOut": {
        const { callId } = event.data;
        if (callsState.outgoingCall?.callId === callId) {
          setCallsState("outgoingCall", null);
          addToast("Call timed out — no answer", "info");
        }
        // Refresh missed list — the backend wrote a row for the
        // local user so the badge ticks up.
        void refreshMissedCalls();
        break;
      }
      case "callMissed": {
        const { callId } = event.data;
        setCallsState("incomingCalls", (prev) => prev.filter((c) => c.callId !== callId));
        void refreshMissedCalls();
        break;
      }
      case "callDeclined": {
        const { callId, reason } = event.data;
        if (callsState.outgoingCall?.callId === callId) {
          setCallsState("outgoingCall", null);
        }
        addToast(reason ? `Call declined: ${reason}` : "Call declined", "info");
        break;
      }
      case "callEnded": {
        // C2 hangup — fired by both the local end_dm_call command (via
        // backend emit) and the remote peer's CallEnd payload arrival.
        // Clears the active-call slot uniformly.
        const { callId, reason } = event.data;
        if (callsState.activeCall?.callId === callId) {
          setCallsState("activeCall", null);
        }
        // Also handle the rare case where activeCall hasn't been
        // populated yet because callConnected raced with callEnded.
        if (callsState.outgoingCall?.callId === callId) {
          setCallsState("outgoingCall", null);
        }
        if (reason) {
          addToast(`Call ended: ${reason}`, "info");
        }
        break;
      }
      default:
        // Other ChatEvent variants are owned by other subscribers.
        break;
    }
  });
}

export async function refreshMissedCalls(): Promise<void> {
  try {
    const rows = await commands.getMissedCalls();
    setCallsState("missed", rows.map((r) => ({
      callId: r.callId,
      peerKey: r.peerKey,
      kind: r.kind,
      expiredAt: r.expiredAt,
    })));
  } catch (e) {
    console.error("Failed to refresh missed calls:", e);
  }
}

/// Initiate an outgoing call. The backend returns the `call_id` once
/// the offer has been delivered and (synchronously) the
/// `CallAccept`/`CallDecline` reply has resolved. We seed the
/// `outgoingCall` store entry up-front so the UI shows "Calling…"
/// immediately rather than waiting for the round-trip.
export async function handleStartDmCall(
  peerKey: string,
  displayName: string,
  video: boolean,
): Promise<void> {
  const expiresAtMs = Date.now() + 30_000;
  const seed: CallEntry = {
    callId: "",
    peerKey,
    displayName,
    kind: video ? "video" : "audio",
    expiresAtMs,
    startedAtMs: Date.now(),
  };
  setCallsState("outgoingCall", seed);
  try {
    const callId = await commands.startDmCall(peerKey, video);
    setCallsState("outgoingCall", "callId", callId);
  } catch (e) {
    setCallsState("outgoingCall", null);
    addToast(`Call failed: ${String(e)}`, "error");
  }
}

/// C2 hangup — end an Active call. Notifies the backend which removes
/// from active_calls, sends CallEnd to the peer, and emits CallEnded
/// locally so the listener clears callsState.activeCall.
export async function handleEndDmCall(callId: string, reason?: string): Promise<void> {
  try {
    await commands.endDmCall(callId, reason);
  } catch (e) {
    addToast(`Failed to end call: ${String(e)}`, "error");
    // Backend may have already removed the call (race with callEnded
    // event); clear the local slot defensively so the UI doesn't get
    // stuck showing an "Active" call that's already gone server-side.
    if (callsState.activeCall?.callId === callId) {
      setCallsState("activeCall", null);
    }
  }
}

export async function handleAcceptIncomingCall(callId: string): Promise<void> {
  try {
    await commands.acceptDmCall(callId);
  } catch (e) {
    addToast(`Failed to accept call: ${String(e)}`, "error");
    setCallsState("incomingCalls", (prev) => prev.filter((c) => c.callId !== callId));
  }
}

export async function handleDeclineIncomingCall(
  callId: string,
  reason?: string,
): Promise<void> {
  try {
    await commands.declineDmCall(callId, reason);
  } catch (e) {
    console.error("Failed to decline call:", e);
  }
  setCallsState("incomingCalls", (prev) => prev.filter((c) => c.callId !== callId));
}
