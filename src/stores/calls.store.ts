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
}

const [callsState, setCallsState] = createStore<CallsState>({
  incomingCalls: [],
  outgoingCall: null,
  activeCall: null,
  missed: [],
});

export { callsState, setCallsState };
