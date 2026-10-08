// Call event subscriptions — `chat-event` → `calls.store` dispatch and
// missed-call sync.
//
// The IPC kickers this used to re-export (`handleStartDmCall`,
// `handleAcceptIncomingCall`, `stopActiveRing`, …) now live in
// `src/actions/calls.actions.ts`. Eleven components imported this
// barrel for those, and got the subscription wiring in the same
// import — one name covering two tiers.

export { subscribeCallEvents, refreshMissedCalls } from "./calls/events";
