import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeCommunityEvents } from "../../ipc/channels";
import { isLegacyCommunityEvent } from "../../ipc/channels/community_subscription_events";
import { reduceSubscriptionMembership } from "./dispatcher_members";
import { applySuccessorEvent } from "../../stores/successor.store";

/// The buddy list's community list (lead decision (41)): it hears each
/// community's arrival from the Host, as the community windows do, so a
/// community created or joined off this window's own request (a
/// co-creation's genesis, a co-creator's `CreatorJoin`) is listed exactly
/// as one this window asked for. The Host publishes the arrival
/// (`GovernanceEvent::MetadataChanged`) from its one create and one join
/// path; a community this window does not hold is re-read from the Host
/// (`withCommunity`), and a held one takes the new name and images. It
/// also hears each offered successor (lead decision (33)), for the
/// "moved to" rows beside the co-creator invites.
export function subscribeBuddyListCommunityEvents(): Promise<UnlistenFn> {
  return subscribeCommunityEvents((event) => {
    if (isLegacyCommunityEvent(event)) return;
    // A kick, a ban or a withdrawn join: this window owns their notices
    // (plan JW lead ruling); the other membership events are the community
    // windows' (member lists this window never shows).
    if ("membership" in event) {
      const m = event.membership;
      if ("kicked" in m || "banned" in m || "joinWithdrawn" in m) reduceSubscriptionMembership(event, true);
      return;
    }
    if (!("governance" in event)) return;
    if ("metadataChanged" in event.governance) {
      reduceSubscriptionMembership(event);
    } else {
      applySuccessorEvent(event.governance);
    }
  });
}
