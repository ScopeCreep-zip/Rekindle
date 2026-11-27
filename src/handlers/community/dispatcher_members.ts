import { handleLoadExpressions } from "../../actions/community/lifecycle";
import type { CommunityEvent } from "../../ipc/channels";
import type { CommunitySubscriptionEvent } from "../../ipc/channels/community_subscription_events";
import { commands } from "../../ipc/commands";
import {
  communityState,
  dropCommunity,
  setCommunityMembers,
  setCommunityState,
} from "../../stores/community.store";
import {
  applyJoinProgress,
  type JoinStageStatus,
  resetJoinProgress,
} from "../../stores/join.store";
import { addToast } from "../../stores/toast.store";
import { removalText } from "../../utils/removal_text";
import { transformMember } from "../../utils/transformers";
import { withCommunity } from "./community_reread";
import { applyGovernanceEvent } from "./dispatcher_governance";
import { refreshCommunityMembers, refreshForUnlistedMember } from "./members_refresh";

/// Membership / governance / moderation-alert slice of the community
/// event dispatcher. Returns `true` when the event was consumed.
export function reduceMembership(event: CommunityEvent): boolean {
  if (event.type === "expressionAssetReady") {
    // Architecture §18.4 — eager-fetch landed for this expression;
    // refresh the community's expression list so the picker re-renders
    // with the resolved inline_data_base64 instead of `:emojiname:`.
    void handleLoadExpressions(event.data.communityId);
    return true;
  }
  return false;
}

/// Membership events on the daemon vocabulary. Same store effects as
/// the `{ type, data }` cases they replaced; only the shape changed.
/// `notices`: this window says membership notices (a toast for our own
/// kick, ban or withdrawn join). Only the buddy list does (lead ruling,
/// plan JW): every window takes the state, one says it.
export function reduceSubscriptionMembership(
  event: CommunitySubscriptionEvent,
  notices = false
): void {
  if ("system" in event) {
    return;
  }
  if ("governance" in event) {
    applyGovernanceEvent(event.governance);
    return;
  }
  if (!("membership" in event)) return;
  const m = event.membership;
  const community = membershipRowCommunity(m);
  if (community !== null) {
    // Each of these writes under the community's row: one the window does
    // not hold yet is re-read first (walk 13: a write under a missing row threw).
    withCommunity(community, () => applyMembershipRow(m, notices));
    return;
  }
  applyMembershipNotice(m, notices);
}

type MembershipEvent = Extract<CommunitySubscriptionEvent, { membership: unknown }>["membership"];

/** The community a row-writing membership event names, or null for the others. */
function membershipRowCommunity(m: MembershipEvent): string | null {
  if ("joined" in m) return m.joined.community;
  if ("memberCardChanged" in m) return m.memberCardChanged.community;
  if ("kicked" in m) return m.kicked.community;
  if ("banned" in m) return m.banned.community;
  if ("removed" in m) return m.removed.community;
  if ("left" in m) return m.left.community;
  if ("rolesChanged" in m) return m.rolesChanged.community;
  if ("timeoutStatusChanged" in m) return m.timeoutStatusChanged.community;
  if ("membersRefreshed" in m) return m.membersRefreshed.community;
  if ("memberDiscovered" in m) return m.memberDiscovered.community;
  if ("onboardingCompleted" in m) return m.onboardingCompleted.community;
  return null;
}

/** A membership event against a row the window holds. */
function applyMembershipRow(m: MembershipEvent, notices: boolean): void {
  // A new member: the Host decides its group, so the list is re-read.
  if ("joined" in m) {
    refreshCommunityMembers(m.joined.community);
    return;
  }

  // A member's card was read or changed (plan E3.2 F9-2, E3.4 S6): its
  // name (never empty, the Host's: the card name or the short fingerprint)
  // and custom status, ours included after a set-name or profile update.
  if ("memberCardChanged" in m) {
    const { community, pseudonym, displayName, customStatus } = m.memberCardChanged;
    const c = communityState.communities[community];
    if (c?.myPseudonymKey && c.myPseudonymKey.toLowerCase() === pseudonym.toLowerCase()) {
      setCommunityState("communities", community, {
        myDisplayName: displayName,
        myCustomStatus: customStatus ?? null,
      });
      // The event names the shown name; the set-name form's prefill is the
      // card name as set, which only the Host's view states.
      commands
        .getCommunityDetails()
        .then((details) => {
          const d = details.find((x) => x.id === community);
          if (d) setCommunityState("communities", community, "myCardName", d.myCardName ?? null);
        })
        .catch(() => {});
    }
    const idx = (c?.members ?? []).findIndex((x) => x.pseudonymKey === pseudonym);
    if (idx >= 0) {
      setCommunityState("communities", community, "members", idx, {
        displayName,
        customStatus: customStatus ?? null,
      });
    } else {
      refreshForUnlistedMember(community, pseudonym);
    }
    return;
  }

  if ("kicked" in m || "banned" in m) {
    const banned = "banned" in m;
    const r = "banned" in m ? m.banned : m.kicked;
    const c = communityState.communities[r.community];
    if (!c) return;
    if (c.myPseudonymKey === r.targetPseudonym) {
      // Our own removal, from the Host's merged governance (plan E3.2 S9):
      // the community stays as read-only history.
      const removed = { banned, by: r.removedBy, byName: r.removedByName, reason: r.reason };
      setCommunityState("communities", r.community, "removed", removed);
      if (notices) addToast(removalText(c.name, removed), "error");
      return;
    }
    setCommunityState("communities", r.community, "members", (prev) =>
      prev.filter((x) => x.pseudonymKey !== r.targetPseudonym)
    );
    return;
  }

  // A leave, from the Host's merged governance (A/B F7b-1), drops the
  // member as a removal does.
  if ("removed" in m || "left" in m) {
    const { community, pseudonym } = "left" in m ? m.left : m.removed;
    setCommunityState("communities", community, "members", (prev) =>
      prev.filter((x) => x.pseudonymKey !== pseudonym)
    );
    return;
  }

  if ("rolesChanged" in m) {
    const { community, pseudonym, roleIds } = m.rolesChanged;
    const c = communityState.communities[community];
    if (c) {
      const idx = c.members.findIndex((x) => x.pseudonymKey === pseudonym);
      if (idx >= 0) {
        setCommunityState("communities", community, "members", idx, "roleIds", roleIds);
      }
      if (pseudonym === c.myPseudonymKey) {
        setCommunityState("communities", community, "myRoleIds", roleIds);
      }
      refreshCommunityMembers(community);
    }
    return;
  }

  if ("timeoutStatusChanged" in m) {
    const { community, pseudonym, timeoutUntil } = m.timeoutStatusChanged;
    const c = communityState.communities[community];
    if (c) {
      const idx = c.members.findIndex((x) => x.pseudonymKey === pseudonym);
      if (idx >= 0) {
        setCommunityState("communities", community, "members", idx, "timeoutUntil", timeoutUntil);
      }
    }
    return;
  }

  if ("membersRefreshed" in m) {
    const { community } = m.membersRefreshed;
    commands
      .getCommunityMembers(community)
      .then((members) => {
        setCommunityMembers(community, members.map(transformMember));
      })
      .catch(() => {});
    return;
  }

  if ("memberDiscovered" in m) {
    refreshCommunityMembers(m.memberDiscovered.community);
    return;
  }

  if ("onboardingCompleted" in m) {
    const { community, pseudonym, roleIds } = m.onboardingCompleted;
    const c = communityState.communities[community];
    const idx = (c?.members ?? []).findIndex((x) => x.pseudonymKey === pseudonym);
    if (idx >= 0) {
      setCommunityState("communities", community, "members", idx, "roleIds", roleIds);
      refreshCommunityMembers(community);
    }
    if (c?.myPseudonymKey === pseudonym) {
      setCommunityState("communities", community, "onboardingComplete", true);
    }
  }
}

/** Membership events that write no community row: join progress, a
 *  withdrawn join, and our own removal (written on no row: the row goes). */
function applyMembershipNotice(m: MembershipEvent, notices: boolean): void {
  if ("forgotten" in m) {
    // The Host forgot the community (LF1): drop its row and, when it was the
    // shown one, the selection. Idempotent with the leave's own drop.
    dropCommunity(m.forgotten.community);
    return;
  }
  // A join the inviter refused after it returned (plan JW, lead ruling):
  // the buddy list drops the community, ends the stepper and says the
  // Host's words verbatim; a community window holding it keeps it and
  // shows the withdrawal in place (its banner, a closed composer), so the
  // user sees why rather than a window going blank.
  if ("joinWithdrawn" in m) {
    const { community, reasonText } = m.joinWithdrawn;
    resetJoinProgress();
    if (notices) {
      dropCommunity(community);
      addToast(reasonText, "error");
    } else if (communityState.communities[community]) {
      setCommunityState("communities", community, "withdrawn", { reasonText });
    }
    return;
  }
  if ("joinProgress" in m) {
    // Pure display: mirror the phase into the join store so the
    // stepper re-renders. No timeout logic — the backend gate owns
    // each phase's budget.
    const { stage, status } = m.joinProgress;
    applyJoinProgress(stage, status as JoinStageStatus);
  }
}

/// Governance events on the daemon vocabulary.
///
/// These carry the new state inline rather than telling the client to
/// re-read it — see the note on Tier 1's `GovernanceEvent`. The one
/// exception is `governanceRebuilt`, which changes channels, roles,
/// members and permissions together and so still triggers a re-read.
