import { commands } from "../ipc/commands";
import { addToast } from "../stores/toast.store";
import { produce } from "solid-js/store";
import { callsState, setCallsState } from "../stores/calls.store";
import { localCallGone, localDeclineCall, localDropGroupRing, localReaction, pruneReaction } from "../stores/calls.reduce";
import { stopActiveRing } from "./calls_ring";
import { errorMessage } from "../utils/error";

/// Initiate an outgoing call. The backend returns the `call_id` once
/// the offer has been delivered and (synchronously) the
/// `CallAccept`/`CallDecline` reply has resolved. We seed the
/// `outgoingCall` store entry up-front so the UI shows "Calling…"
/// immediately rather than waiting for the round-trip.
///
/// Wave 15 W15.6 — thin IPC kicker. Backend's start_dm_call emits the
/// authoritative ChatEvent::CallStarted which seeds the outgoingCall
/// store + starts ringback in the case "callStarted" arm above. No
/// local seed math, no per-frontend policy. Tauri / CLI / TUI all
/// react identically from the same event stream.
///
/// `displayName` arg is unused now (backend resolves via friend_display_name)
/// but kept for source compatibility with the buddy-list / chat header
/// call sites until those are pruned.
export async function handleStartDmCall(
  peerKey: string,
  _displayName: string,
  video: boolean,
): Promise<void> {
  try {
    await commands.startDmCall(peerKey, video);
    // Backend emits ChatEvent::CallStarted; the case arm seeds
    // outgoingCall + ringback. No local state mutation here.
  } catch (e) {
    addToast(errorMessage(e), "error");
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
    // The end succeeds with the Host's `ended`; a refusal means the call is
    // already gone (a race with its `ended`), so no slot keeps showing it.
    addToast(errorMessage(e), "error");
    setCallsState(produce((s) => localCallGone(s, callId)));
  }
}

/// Wave 12 W12.9 — start a group call. Backend fans out a per-recipient
/// wrapped call_key to every invitee and returns the call_id once
/// offers are dispatched. Replies arrive asynchronously as
/// chat-event::groupCallParticipantJoined / Left.
export async function handleStartGroupCall(
  participantPubkeys: string[],
  video: boolean,
  groupId: string | null = null,
): Promise<string | null> {
  if (participantPubkeys.length === 0) {
    addToast("Group call needs at least one invitee", "error");
    return null;
  }
  try {
    const callId = await commands.startGroupCall(participantPubkeys, video, groupId);
    return callId;
  } catch (e) {
    // The Host's refusal in its words (a participant outside the group's
    // roster, GV2), never "[object Object]".
    addToast(`Group call failed: ${errorMessage(e)}`, "error");
    return null;
  }
}

export async function handleAcceptGroupCall(callId: string): Promise<void> {
  stopActiveRing();
  try {
    await commands.acceptGroupCall(callId);
  } catch (e) {
    addToast(errorMessage(e), "error");
    setCallsState(produce((s) => localDropGroupRing(s, callId)));
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
    addToast(errorMessage(e), "error");
  }
  // GroupCallState answers a decline with no event until it is the Host's (S7).
  setCallsState(produce((s) => localDropGroupRing(s, callId)));
}

export async function handleEndGroupCall(callId: string, reason?: string): Promise<void> {
  stopActiveRing();
  try {
    await commands.endGroupCall(callId, reason);
  } catch (e) {
    addToast(errorMessage(e), "error");
    setCallsState(produce((s) => localCallGone(s, callId)));
  }
}

/// Wave 12 W12.11 — fire a reaction at the active call peer. Pushes the
/// emoji into our own `recentReactions` immediately for instant local
/// feedback, then sends the envelope. Loss is tolerable.
export async function handleSendCallReaction(emoji: string): Promise<void> {
  const active = callsState.activeCall;
  if (!active) return;
  setCallsState(produce((s) => localReaction(s, emoji, Date.now())));
  try {
    await commands.sendCallReaction(active.callId, emoji);
  } catch (e) {
    addToast(errorMessage(e), "error");
  }
}

/// Wave 12 W12.11 — remove a reaction once its float animation finished.
/// Called by ReactionFloater on each glyph's `animationend`.
export function removeCallReaction(id: string): void {
  setCallsState(produce((s) => pruneReaction(s, id)));
}

export async function handleAcceptIncomingCall(callId: string): Promise<void> {
  // Stop the ring immediately on click; the backend round-trip to derive
  // the X25519 shared key takes a few hundred ms and we don't want to keep
  // ringing through the handshake.
  stopActiveRing();
  try {
    await commands.acceptDmCall(callId);
  } catch (e) {
    // The accept succeeds with the Host's `connected`; a refused one drops
    // the ring, which no event will now answer.
    addToast(errorMessage(e), "error");
    setCallsState(produce((s) => localDeclineCall(s, callId)));
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
    addToast(errorMessage(e), "error");
  }
  // The desktop path emits nothing for our decline until the call machine
  // is the Host's (S7), so the ring goes here.
  setCallsState(produce((s) => localDeclineCall(s, callId)));
}

// Stopping the ringtone is a user action, so it belongs on this side of
// the split rather than in the event-subscription module.
export { stopActiveRing } from "./calls_ring";
