// User-action dispatchers for communities.
//
// Split out of `handlers/community.handlers.ts`, which re-exported both
// these and `subscribeCommunityEventDispatcher` — two different tiers
// under one name. A component calling `handleAddReaction` on click is
// ordinary; a component importing event-subscription wiring is not, and
// the barrel made them indistinguishable at the import site.
//
// See `.dependency-cruiser.cjs`: components and windows may import
// `src/actions/`; only the app bootstrap imports `src/handlers/`.

export * from "./community/channels";
export * from "./community/events_threads";
export * from "./community/invites";
export * from "./community/lifecycle";
export * from "./community/messages";
export * from "./community/moderation";
export * from "./community/profile";
export * from "./community/roles";
export { typingUsers } from "./community/shared";
