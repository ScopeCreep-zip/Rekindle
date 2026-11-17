import { setCommunityState, communityState } from "../../stores/community.store";
import { commands } from "../../ipc/commands";
import { transformMember } from "../../utils/transformers";

// The member list's groups are the Host's (`CommunityMemberView.group`), so
// any change that can move a member between groups (presence, role
// assignment, role edits, a new member) re-reads the list. Changes that land
// while a read is in flight coalesce into one more read.
const inFlight = new Set<string>();
const dirty = new Set<string>();

/** Re-read `community`'s member list from the Host. */
export function refreshCommunityMembers(community: string): void {
  if (!communityState.communities[community]) return;
  if (inFlight.has(community)) {
    dirty.add(community);
    return;
  }
  inFlight.add(community);
  commands
    .getCommunityMembers(community)
    .then((members) => {
      if (communityState.communities[community]) {
        setCommunityState("communities", community, "members", members.map(transformMember));
      }
    })
    .catch((e) => console.error(`Failed to refresh members for ${community}:`, e))
    .finally(() => {
      inFlight.delete(community);
      if (dirty.delete(community)) refreshCommunityMembers(community);
    });
}
