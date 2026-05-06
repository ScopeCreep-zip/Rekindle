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
