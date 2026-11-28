import { commands } from "../../ipc/commands";
import {
  communityState,
  markMembersFailed,
  setCommunityMembers,
} from "../../stores/community.store";
import { errorMessage } from "../../utils/error";
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
        setCommunityMembers(community, members.map(transformMember));
        const asked = askedFor.get(community);
        for (const m of members) asked?.delete(m.pseudonymKey);
      }
    })
    .catch((e) => {
      console.error(`Failed to refresh members for ${community}:`, e);
      markMembersFailed(community, errorMessage(e));
    })
    .finally(() => {
      inFlight.delete(community);
      if (dirty.delete(community)) refreshCommunityMembers(community);
    });
}

// Members an event named while the list lacked them, per community: each
// is re-read for once, until a read lists it (B61: a member's presence or
// card can arrive before the list holds the member).
const askedFor = new Map<string, Set<string>>();
const pending = new Set<string>();
/** Events for unlisted members within this window share one read. */
const UNLISTED_DEBOUNCE_MS = 250;

/**
 * An event named `member` of `community`, which the member list does not
 * hold: re-read the list once for it, as a `Joined` does, so the member
 * appears with the Host's status and name instead of never.
 */
export function refreshForUnlistedMember(community: string, member: string): void {
  if (!communityState.communities[community]) return;
  let asked = askedFor.get(community);
  if (!asked) {
    asked = new Set();
    askedFor.set(community, asked);
  }
  if (asked.has(member)) return;
  asked.add(member);
  if (pending.has(community)) return;
  pending.add(community);
  setTimeout(() => {
    pending.delete(community);
    refreshCommunityMembers(community);
  }, UNLISTED_DEBOUNCE_MS);
}
