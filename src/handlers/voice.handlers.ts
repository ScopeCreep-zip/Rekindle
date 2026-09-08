// Buddy-list voice presence subscription.
//
// `initVoiceEventListener` moved to `src/actions/voice.actions.ts`: it is
// session-scoped, not app-start wiring — `handleJoinVoice` starts it and
// `handleLeaveVoice` tears it down, and it owns the unlisten handle they
// both touch. Splitting the handle from its two callers was the only
// thing standing in the way of an acyclic tier order.

import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeVoiceEvents } from "../ipc/channels";
import { friendsState, setFriendsState } from "../stores/friends.store";

export function subscribeBuddyListVoiceEvents(): Promise<UnlistenFn> {
  return subscribeVoiceEvents((event) => {
    switch (event.type) {
      case "userJoined": {
        if (friendsState.friends[event.data.publicKey]) {
          setFriendsState("friends", event.data.publicKey, "voiceChannel", "active");
        }
        break;
      }
      case "userLeft": {
        if (friendsState.friends[event.data.publicKey]) {
          setFriendsState("friends", event.data.publicKey, "voiceChannel", null);
        }
        break;
      }
    }
  });
}
