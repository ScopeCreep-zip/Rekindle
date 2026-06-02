// Direct/group call handlers — facade over the `calls/` submodules.
//
// - `calls/ring` — single in-flight ringtone handle (`stopActiveRing`)
// - `calls/events` — `chat-event` → `calls.store` dispatch + missed-call sync
// - `calls/actions` — IPC kickers invoked by UI (start/end/accept/decline/react)

export { stopActiveRing } from "./calls/ring";
export { subscribeCallEvents, refreshMissedCalls } from "./calls/events";
export {
  handleStartDmCall,
  handleEndDmCall,
  handleStartGroupCall,
  handleAcceptGroupCall,
  handleDeclineGroupCall,
  handleEndGroupCall,
  handleSendCallReaction,
  removeCallReaction,
  handleAcceptIncomingCall,
  handleDeclineIncomingCall,
} from "./calls/actions";
