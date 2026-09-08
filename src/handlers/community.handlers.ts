// Community event-subscription wiring.
//
// The action dispatchers this used to re-export now live in
// `src/actions/community.actions.ts`. Only the app bootstrap should
// import this module; a component that wants to *do* something reaches
// for the actions barrel instead.

export { subscribeCommunityEventDispatcher } from "./community/dispatcher";
