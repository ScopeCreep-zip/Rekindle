import { createStore } from "solid-js/store";

/** Plan §Failure 5 — frontend mirror of backend call state. The backend
 *  owns truth (state.active_calls); the store is rebuilt entirely from
 *  ChatEvent emits so tearing it down on logout is just `setCallsState`
 *  back to defaults. */

export interface CallEntry {
  callId: string;
  peerKey: string;
  displayName: string;
  kind: "audio" | "video";
  expiresAtMs: number;
  startedAtMs: number;
  /** Wave 12 W12.6 — peer's last-known media flags. Updated by
   *  chat-event::callMediaStateChanged. UI mounts video/screen tiles
   *  in response. Undefined means "no ping received yet"; default
   *  assumption is `audio: true, video: kind === "video", screen: false`
   *  so the tile shows immediately for a video call without waiting
   *  for the first ping. */
  peerMediaState?: {
    audio: boolean;
    video: boolean;
    screen: boolean;
    timestampMs: number;
  };
}

export interface CallReactionFloat {
  /** Unique key so the floater renders distinct nodes per reaction. */
  id: string;
  emoji: string;
  /** "us" if the local user fired it, otherwise the peer's pubkey. */
  sender: string;
  timestampMs: number;
}

/// Wave 12 W12.9 — frontend mirror of GroupCallState.
export interface GroupCallEntry {
  callId: string;
  /** Pubkey of whoever started the call. Equal to local pubkey when
   *  we're the initiator. */
  initiatorKey: string;
  /** Friendly name to render in headers. */
  displayName: string;
  kind: "audio" | "video";
  /** All invited participants (hex Ed25519). Includes the initiator. */
  participants: string[];
  /** Subset that have accepted so far. */
  accepted: string[];
  startedAtMs: number;
  expiresAtMs: number;
}

export interface CallsState {
  /** Calls awaiting the local user's accept/decline decision. The
   *  IncomingCallModal reads `incomingCalls[0]` so multiple
   *  simultaneous offers are queued in arrival order. */
  incomingCalls: CallEntry[];
  /** Outgoing call still ringing — only one can exist at a time
   *  because `start_dm_call` returns the call_id synchronously. */
  outgoingCall: CallEntry | null;
  /** Connected call (post-handshake). Same single-slot constraint. */
  activeCall: CallEntry | null;
  /** Missed call rows from SQLite. Refreshed on login + on every
   *  `callMissed` / `callTimedOut` event. */
  missed: { callId: string; peerKey: string; kind: number; expiredAt: number }[];
  /** Wave 12 W12.11 — short-lived emoji reactions. Pushed on receive
   *  and on local fire; pruned by ReactionFloater when the float
   *  animation completes. */
  recentReactions: CallReactionFloat[];
  /** Wave 12 W12.9 — incoming group call ringing the local user. */
  incomingGroupCalls: GroupCallEntry[];
  /** Wave 12 W12.9 — currently-active group call (we accepted or
   *  initiated and at least one peer accepted). */
  activeGroupCall: GroupCallEntry | null;
}

const [callsState, setCallsState] = createStore<CallsState>({
  incomingCalls: [],
  outgoingCall: null,
  activeCall: null,
  missed: [],
  recentReactions: [],
  incomingGroupCalls: [],
  activeGroupCall: null,
});

export { callsState, setCallsState };
