import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeChatEvents } from "../ipc/channels";
import { callsState, setCallsState, type CallEntry } from "../stores/calls.store";
import { commands } from "../ipc/commands";
import { addToast } from "../stores/toast.store";
import { settingsState } from "../stores/settings.store";
import { setNotificationState } from "../stores/notification.store";
import { friendsState } from "../stores/friends.store";
import {
  playBusyTone,
  playIncomingRing,
  playOutgoingRingback,
  type RingHandle,
} from "../utils/ringtone";

/// Wave 12 W12.8 — push a `missed_call` row into the notification inbox
/// so the user sees a Call-back / Send Message action even after the
/// call has terminated. Looks up the friend's display name from the
/// friends store; falls back to a truncated pubkey otherwise.
function pushMissedCallNotification(
  callId: string,
  peerKey: string,
  kind: "audio" | "video",
  outgoing: boolean,
): void {
  const friend = friendsState.friends[peerKey];
  const name = friend?.displayName ?? peerKey.slice(0, 12) + "…";
  setNotificationState("notifications", (prev) => [
    ...prev,
    {
      id: crypto.randomUUID(),
      type: "missed_call",
      title: outgoing ? "Call not answered" : "Missed call",
      body: `${name} (${kind})`,
      timestamp: Date.now(),
      read: false,
      callId,
      peerKey,
      callKind: kind,
    },
  ]);
  setNotificationState("unreadCount", (c) => c + 1);
}

// Wave 12 W12.1 — single in-flight ring handle per webview. Replaced
// when a new ring would start (e.g. caller declined while another offer
// already arrived); cleared on every terminal call event.
let activeRing: RingHandle | null = null;
function stopActiveRing(): void {
  if (activeRing != null) {
    activeRing.stop();
    activeRing = null;
  }
}
export { stopActiveRing };

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
        // W12-fix.B — ring even when the window is hidden / unfocused;
        // hearing the call is the WHOLE point. Web Audio plays from
        // background webviews fine. Multi-window overlap is acceptable
        // (you'll hear the ring louder); the alternative — silent ring
        // when the window isn't focused — is unusable.
        // - Already-in-call gets the call-waiting beep instead of a full
        //   ring (Discord/Telegram convention).
        // - User can disable ringtone entirely via settingsState.
        if (!settingsState.ringtoneEnabled) break;
        stopActiveRing();
        if (callsState.activeCall != null) {
          activeRing = playBusyTone({ volume: settingsState.ringtoneVolume * 0.6 });
        } else {
          activeRing = playIncomingRing({ volume: settingsState.ringtoneVolume });
        }
        break;
      }
      case "callConnected": {
        const { callId } = event.data;
        stopActiveRing();
        // W13.11 — under the new fire-and-forget signaling, the
        // backend's start_dm_call returns the call_id synchronously
        // BEFORE any callConnected event can fire (callConnected
        // requires a CallAccept envelope round-trip; that takes
        // strictly longer than the local IPC return). The W12-fix.D
        // empty-string-fallback shim is no longer needed; matching by
        // exact callId is enough.
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
      case "callRinging": {
        // W13 — receiver acknowledged our invite and is ringing the
        // user. Flip the OutgoingCallPanel label from "Calling…" to
        // "Ringing…" via a status field on the entry.
        const { callId } = event.data;
        const out = callsState.outgoingCall;
        if (out && out.callId === callId) {
          setCallsState("outgoingCall", "status", "ringing");
        }
        break;
      }
      case "callTimedOut": {
        const { callId } = event.data;
        stopActiveRing();
        const out = callsState.outgoingCall;
        if (out?.callId === callId) {
          pushMissedCallNotification(out.callId, out.peerKey, out.kind, true);
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
        stopActiveRing();
        const idx = callsState.incomingCalls.findIndex((c) => c.callId === callId);
        if (idx >= 0) {
          const entry = callsState.incomingCalls[idx];
          pushMissedCallNotification(entry.callId, entry.peerKey, entry.kind, false);
          setCallsState("incomingCalls", (prev) => prev.filter((_, i) => i !== idx));
        }
        void refreshMissedCalls();
        break;
      }
      case "callDeclined": {
        const { callId, reason } = event.data;
        stopActiveRing();
        if (callsState.outgoingCall?.callId === callId) {
          setCallsState("outgoingCall", null);
        }
        addToast(reason ? `Call declined: ${reason}` : "Call declined", "info");
        break;
      }
      case "incomingGroupCall": {
        // Wave 12 W12.9 — incoming group call. Push into the queue;
        // GroupCallPanel reads incomingGroupCalls[0].
        const { callId, from, displayName, kind, participants, expiresAtMs } = event.data;
        setCallsState("incomingGroupCalls", (prev) => [
          ...prev,
          {
            callId,
            initiatorKey: from,
            displayName,
            kind,
            participants,
            accepted: [],
            startedAtMs: Date.now(),
            expiresAtMs,
          },
        ]);
        // W12-fix.B — always ring (background or foreground). The
        // window-focus path is handled by the backend bringing the
        // window forward (W12-fix.C); the ring is the audio cue.
        if (!settingsState.ringtoneEnabled) break;
        stopActiveRing();
        activeRing = playIncomingRing({ volume: settingsState.ringtoneVolume });
        break;
      }
      case "groupCallConnected": {
        // Wave 12 W12.9 — promote a group call to active.
        const { callId } = event.data;
        stopActiveRing();
        const head = callsState.incomingGroupCalls.find((c) => c.callId === callId);
        if (head) {
          setCallsState("activeGroupCall", { ...head, accepted: [head.initiatorKey] });
          setCallsState("incomingGroupCalls", (p) => p.filter((c) => c.callId !== callId));
        }
        break;
      }
      case "groupCallParticipantJoined": {
        const { callId, participantPubkey } = event.data;
        const cur = callsState.activeGroupCall;
        if (cur && cur.callId === callId && !cur.accepted.includes(participantPubkey)) {
          setCallsState("activeGroupCall", "accepted", (a) => [...a, participantPubkey]);
        }
        break;
      }
      case "groupCallParticipantLeft": {
        const { callId, participantPubkey } = event.data;
        const cur = callsState.activeGroupCall;
        if (cur && cur.callId === callId) {
          setCallsState("activeGroupCall", "accepted", (a) =>
            a.filter((p) => p !== participantPubkey),
          );
        }
        break;
      }
      case "groupCallEnded": {
        const { callId, reason } = event.data;
        stopActiveRing();
        if (callsState.activeGroupCall?.callId === callId) {
          setCallsState("activeGroupCall", null);
        }
        setCallsState("incomingGroupCalls", (p) => p.filter((c) => c.callId !== callId));
        if (reason) addToast(`Group call ended: ${reason}`, "info");
        break;
      }
      case "callReactionReceived": {
        // Wave 12 W12.11 — peer fired an emoji reaction. Push to the
        // recent-reactions list with a fresh id; ReactionFloater
        // animates and clears after the float window.
        const { callId, sender, emoji, timestampMs } = event.data;
        if (callsState.activeCall?.callId !== callId) break;
        setCallsState("recentReactions", (prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            emoji,
            sender,
            timestampMs,
          },
        ]);
        break;
      }
      case "callMediaStateChanged": {
        // Wave 12 W12.6 — peer flipped audio/video/screen. Drop if our
        // local last-update is newer (last-write-wins per timestamp).
        const { callId, audio, video, screen, timestampMs } = event.data;
        const updateSlot = (
          slot: "activeCall" | "outgoingCall",
          entry: CallEntry | null,
        ): void => {
          if (entry == null || entry.callId !== callId) return;
          const cur = entry.peerMediaState;
          if (cur && cur.timestampMs >= timestampMs) return;
          setCallsState(slot, "peerMediaState", {
            audio,
            video,
            screen,
            timestampMs,
          });
        };
        updateSlot("activeCall", callsState.activeCall);
        updateSlot("outgoingCall", callsState.outgoingCall);
        const idx = callsState.incomingCalls.findIndex((c) => c.callId === callId);
        if (idx >= 0) {
          const cur = callsState.incomingCalls[idx].peerMediaState;
          if (!cur || cur.timestampMs < timestampMs) {
            setCallsState("incomingCalls", idx, "peerMediaState", {
              audio,
              video,
              screen,
              timestampMs,
            });
          }
        }
        break;
      }
      case "callEnded": {
        // C2 hangup — fired by both the local end_dm_call command (via
        // backend emit) and the remote peer's CallEnd payload arrival.
        // Clears the active-call slot uniformly.
        const { callId, reason } = event.data;
        stopActiveRing();
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
///
/// Wave 12 W12.1 — also starts the synthesized ringback so the caller
/// hears feedback while the offer is in flight. Stopped on connect /
/// decline / timeout / cancel.
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
    status: "calling",
  };
  setCallsState("outgoingCall", seed);
  if (settingsState.ringtoneEnabled) {
    stopActiveRing();
    activeRing = playOutgoingRingback({ volume: settingsState.ringtoneVolume });
  }
  try {
    const callId = await commands.startDmCall(peerKey, video);
    setCallsState("outgoingCall", "callId", callId);
  } catch (e) {
    stopActiveRing();
    setCallsState("outgoingCall", null);
    addToast(`Call failed: ${String(e)}`, "error");
  }
}

/// C2 hangup — end an Active call. Notifies the backend which removes
/// from active_calls, sends CallEnd to the peer, and emits CallEnded
/// locally so the listener clears callsState.activeCall.
///
/// Also serves as the Cancel path for outgoing calls (W12.4) — the
/// backend `end_dm_call` handler covers both the post-connect hangup
/// and the pre-accept cancel cases.
export async function handleEndDmCall(callId: string, reason?: string): Promise<void> {
  stopActiveRing();
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
    if (callsState.outgoingCall?.callId === callId) {
      setCallsState("outgoingCall", null);
    }
  }
}

/// Wave 12 W12.9 — start a group call. Backend fans out a per-recipient
/// wrapped call_key to every invitee and returns the call_id once
/// offers are dispatched. Replies arrive asynchronously as
/// chat-event::groupCallParticipantJoined / Left.
export async function handleStartGroupCall(
  participantPubkeys: string[],
  video: boolean,
): Promise<string | null> {
  if (participantPubkeys.length === 0) {
    addToast("Group call needs at least one invitee", "error");
    return null;
  }
  try {
    const callId = await commands.startGroupCall(participantPubkeys, video);
    return callId;
  } catch (e) {
    addToast(`Group call failed: ${String(e)}`, "error");
    return null;
  }
}

export async function handleAcceptGroupCall(callId: string): Promise<void> {
  stopActiveRing();
  try {
    await commands.acceptGroupCall(callId);
  } catch (e) {
    addToast(`Failed to accept group call: ${String(e)}`, "error");
    setCallsState("incomingGroupCalls", (p) => p.filter((c) => c.callId !== callId));
  }
}

export async function handleDeclineGroupCall(
  callId: string,
  reason?: string,
): Promise<void> {
  stopActiveRing();
  try {
    await commands.declineGroupCall(callId, reason);
  } catch (e) {
    console.error("decline group call:", e);
  }
  setCallsState("incomingGroupCalls", (p) => p.filter((c) => c.callId !== callId));
}

export async function handleEndGroupCall(callId: string, reason?: string): Promise<void> {
  stopActiveRing();
  try {
    await commands.endGroupCall(callId, reason);
  } catch (e) {
    addToast(`Failed to end group call: ${String(e)}`, "error");
    if (callsState.activeGroupCall?.callId === callId) {
      setCallsState("activeGroupCall", null);
    }
  }
}

/// Wave 12 W12.11 — fire a reaction at the active call peer. Pushes the
/// emoji into our own `recentReactions` immediately for instant local
/// feedback, then sends the envelope. Loss is tolerable.
export async function handleSendCallReaction(emoji: string): Promise<void> {
  const active = callsState.activeCall;
  if (!active) return;
  setCallsState("recentReactions", (prev) => [
    ...prev,
    {
      id: crypto.randomUUID(),
      emoji,
      sender: "us",
      timestampMs: Date.now(),
    },
  ]);
  try {
    await commands.sendCallReaction(active.callId, emoji);
  } catch (e) {
    console.warn("sendCallReaction failed:", e);
  }
}

/// Wave 12 W12.11 — remove a reaction once its float animation finished.
/// Called by ReactionFloater on each glyph's `animationend`.
export function removeCallReaction(id: string): void {
  setCallsState("recentReactions", (prev) => prev.filter((r) => r.id !== id));
}

export async function handleAcceptIncomingCall(callId: string): Promise<void> {
  // Stop the ring immediately on click; the backend round-trip to derive
  // the X25519 shared key takes a few hundred ms and we don't want to keep
  // ringing through the handshake.
  stopActiveRing();
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
  stopActiveRing();
  try {
    await commands.declineDmCall(callId, reason);
  } catch (e) {
    console.error("Failed to decline call:", e);
  }
  setCallsState("incomingCalls", (prev) => prev.filter((c) => c.callId !== callId));
}
