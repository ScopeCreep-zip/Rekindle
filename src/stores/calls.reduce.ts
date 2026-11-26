// The calls store's reducers (plan E4.1 S1): the store renders the Host's
// call events and the call window's snapshot; every change to it goes
// through one of these pure functions over a draft of `CallsState`.
//
// A reducer changes state only. What the user should hear or see because of
// an event (a ring, a toast in the Host's words, a missed-call row, the
// conversation brought forward, the camera's policy) comes back as a list
// of effects, which the subscription runs in the window that owns them.
//
// Local inputs (`local*`) are the user's own acts that the call path does
// not answer with an event on this base: a 1:1 decline (the desktop path
// emits nothing for it until the call machine is the Host's, S7), the group
// call's accept/decline/end (the desktop's GroupCallState until S7), a
// refused end, and our own reactions.

import type { CallEvent } from "../ipc/channels/chat_events";
import type { ActiveCall } from "../ipc/commands/types";
import type { CallEntry, CallsState, GroupCallEntry } from "./calls.store";

/** What an event asks of the window, beyond its state. */
export type CallEffect =
  | { kind: "ring"; tone: "incoming" | "ringback" | "busy" }
  | { kind: "stopRing" }
  | { kind: "toast"; text: string }
  | { kind: "missedNotification"; callId: string; peerKey: string; callKind: "audio" | "video"; outgoing: boolean }
  | { kind: "refreshMissed" }
  | { kind: "focusConversation"; peerKey: string }
  | { kind: "cameraOn" };

/** What a reducer reads besides the event: the clock and who we are. */
export interface CallContext {
  nowMs: number;
  selfKey: string;
  selfName: string;
}

const callKind = (kind: string): "audio" | "video" => (kind === "video" ? "video" : "audio");

/** Drop `callId` from every slot that can hold it. */
function clearCall(s: CallsState, callId: string): void {
  if (s.activeCall?.callId === callId) s.activeCall = null;
  if (s.outgoingCall?.callId === callId) s.outgoingCall = null;
  if (s.activeGroupCall?.callId === callId) s.activeGroupCall = null;
  if (s.outgoingGroupCall?.callId === callId) s.outgoingGroupCall = null;
  s.incomingCalls = s.incomingCalls.filter((c) => c.callId !== callId);
  s.incomingGroupCalls = s.incomingGroupCalls.filter((c) => c.callId !== callId);
}

/** One call event from the Host, applied to `s`; returns its effects. */
export function reduceCallEvent(s: CallsState, call: CallEvent, ctx: CallContext): CallEffect[] {
  if ("started" in call) {
    const { callId, kind, peerKey, peerDisplayName, expiresAtMs } = call.started;
    s.outgoingCall = { callId, peerKey, displayName: peerDisplayName, kind: callKind(kind), expiresAtMs, startedAtMs: ctx.nowMs, status: "calling" };
    return [{ kind: "ring", tone: "ringback" }];
  }
  if ("groupStarted" in call) {
    const { callId, kind, participants, groupId, expiresAtMs } = call.groupStarted;
    s.outgoingGroupCall = {
      callId,
      initiatorKey: ctx.selfKey,
      displayName: ctx.selfName,
      kind: callKind(kind),
      participants,
      groupId,
      accepted: [],
      startedAtMs: ctx.nowMs,
      expiresAtMs,
    };
    return [{ kind: "ring", tone: "ringback" }];
  }
  if ("incoming" in call) {
    const { callId, from, displayName, kind, participants, isGroup, groupId, expiresAtMs } = call.incoming;
    if (isGroup) {
      const entry: GroupCallEntry = {
        callId,
        initiatorKey: from,
        displayName,
        kind: callKind(kind),
        participants,
        groupId,
        accepted: [],
        startedAtMs: ctx.nowMs,
        expiresAtMs,
      };
      s.incomingGroupCalls = [...s.incomingGroupCalls, entry];
      return [{ kind: "ring", tone: "incoming" }];
    }
    const entry: CallEntry = { callId, peerKey: from, displayName, kind: callKind(kind), expiresAtMs, startedAtMs: ctx.nowMs };
    // Already in a call: the call-waiting beep, not a full ring.
    const tone = s.activeCall != null ? "busy" : "incoming";
    s.incomingCalls = [...s.incomingCalls, entry];
    return [{ kind: "ring", tone }];
  }
  if ("connected" in call) {
    const { callId, direct, groupId } = call.connected;
    const effects: CallEffect[] = [{ kind: "stopRing" }];
    if (direct == null) {
      // A group call has no single peer: our own ring answered, or one we accepted.
      const ours = s.outgoingGroupCall;
      if (ours && ours.callId === callId) {
        s.activeGroupCall = { ...ours, groupId };
        s.outgoingGroupCall = null;
        return effects;
      }
      const ring = s.incomingGroupCalls.find((c) => c.callId === callId);
      if (ring) {
        s.activeGroupCall = { ...ring, groupId, accepted: [ring.initiatorKey] };
        s.incomingGroupCalls = s.incomingGroupCalls.filter((c) => c.callId !== callId);
      }
      return effects;
    }
    const out = s.outgoingCall;
    if (out && out.callId === callId) {
      s.activeCall = out;
      s.outgoingCall = null;
    } else {
      const entry = s.incomingCalls.find((c) => c.callId === callId);
      if (entry) {
        s.activeCall = entry;
        s.incomingCalls = s.incomingCalls.filter((c) => c.callId !== callId);
      }
    }
    if (direct.expectedLocalCamera) effects.push({ kind: "cameraOn" });
    return effects;
  }
  if ("ringing" in call) {
    if (s.outgoingCall && s.outgoingCall.callId === call.ringing.callId) s.outgoingCall.status = "ringing";
    return [];
  }
  if ("timedOut" in call) {
    const { callId } = call.timedOut;
    const effects: CallEffect[] = [{ kind: "stopRing" }];
    const out = s.outgoingCall;
    if (out?.callId === callId) {
      effects.push({ kind: "missedNotification", callId, peerKey: out.peerKey, callKind: out.kind, outgoing: true });
      s.outgoingCall = null;
      effects.push({ kind: "toast", text: "Call timed out — no answer" });
    }
    effects.push({ kind: "refreshMissed" });
    return effects;
  }
  if ("missed" in call) {
    const { callId } = call.missed;
    const effects: CallEffect[] = [{ kind: "stopRing" }];
    const entry = s.incomingCalls.find((c) => c.callId === callId);
    if (entry) {
      effects.push({ kind: "missedNotification", callId, peerKey: entry.peerKey, callKind: entry.kind, outgoing: false });
      s.incomingCalls = s.incomingCalls.filter((c) => c.callId !== callId);
    }
    effects.push({ kind: "refreshMissed" });
    return effects;
  }
  if ("declined" in call) {
    const { callId, reason } = call.declined;
    clearCall(s, callId);
    return [{ kind: "stopRing" }, { kind: "toast", text: reason ? `Call declined: ${reason}` : "Call declined" }];
  }
  if ("unreachable" in call) {
    // Our invite never reached the peer: no missed-call row (they never saw it).
    const { callId, reason } = call.unreachable;
    clearCall(s, callId);
    return [{ kind: "stopRing" }, { kind: "toast", text: reason ? `Call didn't go through: ${reason}` : "Call didn't go through" }];
  }
  if ("focusConversation" in call) {
    return [{ kind: "focusConversation", peerKey: call.focusConversation.peerKey }];
  }
  if ("participantJoined" in call) {
    const { callId, participantPubkey } = call.participantJoined;
    const cur = s.activeGroupCall;
    if (cur && cur.callId === callId && !cur.accepted.includes(participantPubkey)) cur.accepted = [...cur.accepted, participantPubkey];
    return [];
  }
  if ("participantLeft" in call) {
    const { callId, participantPubkey } = call.participantLeft;
    const cur = s.activeGroupCall;
    if (cur && cur.callId === callId) cur.accepted = cur.accepted.filter((p) => p !== participantPubkey);
    return [];
  }
  if ("reactionReceived" in call) {
    const { callId, sender, emoji, timestampMs } = call.reactionReceived;
    if (s.activeCall?.callId !== callId) return [];
    s.recentReactions = [...s.recentReactions, { id: crypto.randomUUID(), emoji, sender, timestampMs }];
    return [];
  }
  if ("mediaStateChanged" in call) {
    // The peer's toggles, last write wins by its timestamp.
    const { callId, audio, video, screen, timestampMs } = call.mediaStateChanged;
    const media = { audio, video, screen, timestampMs };
    const newer = (e: CallEntry): boolean => !e.peerMediaState || e.peerMediaState.timestampMs < timestampMs;
    for (const e of [s.activeCall, s.outgoingCall, ...s.incomingCalls]) {
      if (e && e.callId === callId && newer(e)) e.peerMediaState = media;
    }
    return [];
  }
  if ("ended" in call) {
    const { callId, reason } = call.ended;
    clearCall(s, callId);
    return reason ? [{ kind: "stopRing" }, { kind: "toast", text: `Call ended: ${reason}` }] : [{ kind: "stopRing" }];
  }
  // `selfMuteChanged` names no call: the Host's voice status carries our mute.
  return [];
}

/** The call window's snapshot of its call (`get_active_call`): connected
 *  is the active call, otherwise the outgoing one. */
export function reduceActiveCallSnapshot(s: CallsState, snapshot: ActiveCall, ctx: CallContext): CallEffect[] {
  const entry: CallEntry = {
    callId: snapshot.callId,
    peerKey: snapshot.peerKey,
    displayName: snapshot.displayName,
    kind: snapshot.kind,
    expiresAtMs: snapshot.expiresAtMs,
    startedAtMs: ctx.nowMs,
  };
  if (snapshot.connected) s.activeCall = entry;
  else s.outgoingCall = entry;
  // The window opens right after `connected`, whose camera policy the
  // snapshot carries as its kind: a video call starts with the camera on.
  return snapshot.connected && snapshot.kind === "video" ? [{ kind: "cameraOn" }] : [];
}

/** We declined a 1:1 ring: the desktop path emits nothing for it until S7. */
export function localDeclineCall(s: CallsState, callId: string): void {
  s.incomingCalls = s.incomingCalls.filter((c) => c.callId !== callId);
}

/** Our end was refused because the call is already gone (a race with its
 *  `ended`): no slot keeps showing it. */
export function localCallGone(s: CallsState, callId: string): void {
  clearCall(s, callId);
}

/** We declined a group ring, or our accept was refused (GroupCallState, S7). */
export function localDropGroupRing(s: CallsState, callId: string): void {
  s.incomingGroupCalls = s.incomingGroupCalls.filter((c) => c.callId !== callId);
}

/** Our reaction, shown at once (its send is best-effort). */
export function localReaction(s: CallsState, emoji: string, nowMs: number): void {
  s.recentReactions = [...s.recentReactions, { id: crypto.randomUUID(), emoji, sender: "us", timestampMs: nowMs }];
}

/** A reaction's float finished. */
export function pruneReaction(s: CallsState, id: string): void {
  s.recentReactions = s.recentReactions.filter((r) => r.id !== id);
}

/** The missed-call rows as the Host stored them. */
export function reduceMissedCalls(s: CallsState, rows: { callId: string; peerKey: string; kind: number; expiredAt: number }[]): void {
  s.missed = rows.map((r) => ({ callId: r.callId, peerKey: r.peerKey, kind: r.kind, expiredAt: r.expiredAt }));
}
