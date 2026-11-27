import { batch } from "solid-js";
import { produce, reconcile } from "solid-js/store";
import type { CommunityEvent } from "../../ipc/channels";
import type {
  CommunitySubscriptionEvent,
  GovernanceEvent,
} from "../../ipc/channels/community_subscription_events";
import { applyJoinProgress, resetJoinProgress, type JoinStageStatus } from "../../stores/join.store";
import { applySuccessorEvent } from "../../stores/successor.store";
import { transformCommunityDetail } from "../../utils/transformers";
import { handleResolveCommunityImageDataUrls } from "../../actions/community/lifecycle";
import { type Community, setCommunityState, setCommunityMembers, communityState, dropCommunity } from "../../stores/community.store";
import { commands } from "../../ipc/commands";
import { addToast } from "../../stores/toast.store";
import { removalText } from "../../utils/removal_text";
import { transformChannel, transformMember } from "../../utils/transformers";
import { refreshCommunityMembers, refreshForUnlistedMember } from "./members_refresh";
import { withCommunity } from "./community_reread";
import { handleLoadExpressions, handleLoadAutoModRules } from "../../actions/community/lifecycle";
import { handleLoadChannelThreads } from "../../actions/community/events_threads";

// A pending rebuild needs the full view even if a later permission event
// supersedes its request. Scope reads to a row so a reply from before a
// leave can't update the replacement row after rejoining.
const permissionReads = new WeakMap<Community, { rebuild: boolean }>();

function readCommunityPermissions(community: string, rebuild: boolean): void {
  const row = communityState.communities[community];
  if (!row) return;
  const read = { rebuild: rebuild || permissionReads.get(row)?.rebuild === true };
  permissionReads.set(row, read);

  void commands
    .getCommunityDetails()
    .then((details) => {
      const detail = details.find((d) => d.id === community);
      if (!detail || communityState.communities[community] !== row || permissionReads.get(row) !== read) return;
      permissionReads.delete(row);
      batch(() => {
        setCommunityState("communities", community, "myPermissions", detail.myPermissions);
        setCommunityState("communities", community, "denied", reconcile(detail.denied ?? {}));
        setCommunityState("communities", community, "myChannelPermissions", detail.myChannelPermissions);
        setCommunityState("communities", community, "weAreCreator", detail.weAreCreator);
        setCommunityState("communities", community, "deviceStatus", detail.deviceStatus);
        if (read.rebuild) {
          setCommunityState("communities", community, {
            name: detail.name,
            description: detail.description ?? null,
            roles: detail.roles ?? [],
            channels: detail.channels.map(transformChannel),
            categories: detail.categories ?? [],
            myRoleIds: detail.myRoleIds,
            mekGeneration: detail.mekGeneration ?? 0,
            removed: detail.removed ?? null,
          });
          return;
        }
        // A permission-only event doesn't replace the rest of a channel.
        for (const ch of detail.channels) {
          setCommunityState(
            "communities", community, "channels", (c) => c.id === ch.id,
            "denied", reconcile(ch.denied ?? {}),
          );
        }
      });
    })
    .catch(() => {});
}

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
  notices = false,
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
      setCommunityState("communities", community, { myDisplayName: displayName, myCustomStatus: customStatus ?? null });
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
      setCommunityState("communities", community, "members", idx, { displayName, customStatus: customStatus ?? null });
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
      prev.filter((x) => x.pseudonymKey !== r.targetPseudonym),
    );
    return;
  }

  // A leave, from the Host's merged governance (A/B F7b-1), drops the
  // member as a removal does.
  if ("removed" in m || "left" in m) {
    const { community, pseudonym } = "left" in m ? m.left : m.removed;
    setCommunityState("communities", community, "members", (prev) =>
      prev.filter((x) => x.pseudonymKey !== pseudonym),
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
function applyGovernanceEvent(g: GovernanceEvent): void {
  // An offered successor is kept by community, not under its row: the
  // offer can arrive before the window holds the community.
  if (applySuccessorEvent(g)) return;
  const community = governanceRowCommunity(g);
  if (community !== null) {
    withCommunity(community, () => applyGovernance(g));
    return;
  }
  applyGovernance(g);
}

/** The community a governance event that writes under its row names, or null. */
function governanceRowCommunity(g: GovernanceEvent): string | null {
  if ("rolesChanged" in g) return g.rolesChanged.community;
  if ("segmentsChanged" in g) return g.segmentsChanged.community;
  if ("channelsChanged" in g) return g.channelsChanged.community;
  if ("metadataChanged" in g) return g.metadataChanged.community;
  if ("channelPermissionsChanged" in g) return g.channelPermissionsChanged.community;
  if ("effectivePermissionsChanged" in g) return g.effectivePermissionsChanged.community;
  if ("governanceRebuilt" in g) return g.governanceRebuilt.community;
  if ("keyStatus" in g) return g.keyStatus.community;
  if ("bansChanged" in g) return g.bansChanged.community;
  return null;
}

/** One governance event; a row-writing one only against a row the window holds. */
function applyGovernance(g: GovernanceEvent): void {
  // FX3: the community key we hold, current or waiting on a key holder.
  if ("keyStatus" in g) {
    const { community, status } = g.keyStatus;
    if (communityState.communities[community]) setCommunityState("communities", community, "keyStatus", status);
    return;
  }
  // The ban list changed (plan E3.2 S9): a shown ban list reads it again
  // from the Host (`bansRevision`), which holds the list.
  if ("bansChanged" in g) {
    const { community } = g.bansChanged;
    if (communityState.communities[community]) {
      setCommunityState("communities", community, "bansRevision", (n) => (n ?? 0) + 1);
    }
    return;
  }
  if ("rolesChanged" in g) {
    const { community, roles } = g.rolesChanged;
    if (communityState.communities[community]) {
      // The store keeps `permissions` as a string and
      // `utils/permissions.ts` parses it with `BigInt(...)`. Tier 1
      // sends a JSON number, which is exact today but is the shape
      // that loses low bits once permission bits pass 2^53 — so the
      // string form is preserved here rather than widening every
      // permission check to accept a number.
      setCommunityState(
        "communities",
        community,
        "roles",
        roles.map((r) => ({
          ...r,
          permissions: String(r.permissions),
          // The store uses `undefined` for absent, Tier 1 uses `null`.
          exclusionGroup: r.exclusionGroup ?? undefined,
        })),
      );
      // A role's hoist or position moves members between groups.
      refreshCommunityMembers(community);
    }
    return;
  }

  if ("segmentsChanged" in g) {
    const { community, segments } = g.segmentsChanged;
    if (communityState.communities[community]) {
      setCommunityState("communities", community, "segments", segments);
    }
    return;
  }

  if ("channelsChanged" in g) {
    const { community, channels, categories } = g.channelsChanged;
    const c = communityState.communities[community];
    if (!c) return;
    // Preserve unread counts from existing channels
    const unreadMap: Record<string, number> = {};
    for (const ch of c.channels) {
      unreadMap[ch.id] = ch.unreadCount;
    }
    setCommunityState(
      "communities",
      community,
      "channels",
      channels.map((ch) => ({
        id: ch.id,
        name: ch.name,
        // Tier 1 calls it `kind`; the store calls it `type`.
        type: ch.kind as "text" | "voice" | "announcement",
        unreadCount: unreadMap[ch.id] ?? 0,
        categoryId: ch.categoryId ?? undefined,
        topic: ch.topic,
        slowmodeSeconds: ch.slowmodeSeconds ?? undefined,
        locked: ch.locked ?? false,
        lockedReason: ch.lockedReason,
      })),
    );
    setCommunityState(
      "communities",
      community,
      "categories",
      categories.map((cat) => ({
        id: cat.id,
        name: cat.name,
        sortOrder: cat.sortOrder,
      })),
    );
    return;
  }

  if ("metadataChanged" in g) {
    const { community, name, description, iconHash, bannerHash } = g.metadataChanged;
    if (name !== null) {
      setCommunityState("communities", community, "name", name);
    }
    if (description !== null) {
      setCommunityState("communities", community, "description", description);
    }
    // Architecture §32 Phase 5 W15 — when the icon/banner hash changes
    // the cached data URL is stale; clear it and re-resolve through the
    // local cache so the buddy-list icon updates.
    if (iconHash !== null) {
      setCommunityState("communities", community, "iconHash", iconHash);
    }
    if (bannerHash !== null) {
      setCommunityState("communities", community, "bannerHash", bannerHash);
    }
    if (iconHash !== null || bannerHash !== null) {
      void handleResolveCommunityImageDataUrls(community);
    }
    return;
  }

  if ("inviteCreated" in g) {
    // Re-read the Host's list, which carries each invite's fingerprint
    // (plan E7.15), when this window shows the invites.
    const community = g.inviteCreated.community;
    if (communityState.communityInvites[community] !== undefined) {
      void commands.listCommunityInvites(community)
        .then((invites) => setCommunityState("communityInvites", community, invites))
        .catch((e) => console.error("[Community] Failed to re-list invites:", e));
    }
    return;
  }

  if ("governanceOpQuarantined" in g) {
    const { community, op, author, reason, cause } = g.governanceOpQuarantined;
    setCommunityState("quarantinedOps", community, (prev) => ({
      ...(prev ?? {}),
      [op]: { author, reason, cause },
    }));
    return;
  }

  if ("checkpointSnapshotMismatch" in g) {
    // Keyed by community, not under its row: the backfill that finds it can
    // finish before the window holds the community.
    setCommunityState("snapshotMismatch", g.checkpointSnapshotMismatch.community, true);
    return;
  }

  if ("opPublication" in g) {
    const { community, op, state } = g.opPublication;
    // A landed op leaves the map; pending and failed ones stay shown. The
    // removal is a `produce` delete: a store setter merges an object it is
    // given into the one it holds, so a copy without the op kept it (walk
    // M86, "Publishing 6 changes" after all six landed).
    if (state.kind !== "landed") {
      setCommunityState("opPublications", community, (prev) => ({ ...(prev ?? {}), [op]: state }));
    } else if (communityState.opPublications[community]?.[op]) {
      setCommunityState("opPublications", community, produce((ops) => { delete ops[op]; }));
    }
    return;
  }

  if ("inviteRevoked" in g || "inviteExpired" in g) {
    const { community, codeHash } = "inviteRevoked" in g ? g.inviteRevoked : g.inviteExpired;
    setCommunityState("communityInvites", community, (prev) =>
      (prev ?? []).filter((inv) => inv.codeHash !== codeHash),
    );
    return;
  }

  if ("inviteUsed" in g) {
    const { community, codeHash, uses } = g.inviteUsed;
    setCommunityState("communityInvites", community, (prev) =>
      (prev ?? []).map((inv) => (inv.codeHash === codeHash ? { ...inv, uses } : inv)),
    );
    return;
  }

  if ("channelPermissionsChanged" in g) {
    const { community, channel } = g.channelPermissionsChanged;
    // A channel whose threads this window shows re-reads them: an overwrite
    // can take away our view, and then the Host lists none (an open thread
    // in it closes; useCommunityWindow).
    if (communityState.channelThreads[channel] !== undefined) {
      void handleLoadChannelThreads(community, channel);
    }
    commands
      .getCommunityDetails()
      .then((details) => {
        const detail = details.find((d: { id: string }) => d.id === community);
        if (detail) {
          setCommunityState("communities", community, "roles", detail.roles);
        }
      })
      .catch(() => {});
    return;
  }

  if ("effectivePermissionsChanged" in g) {
    // Our own permissions moved (plan E3.2 S9): re-read the backend's
    // flags. Another member's change shows through its roles and the
    // member list, which re-render on their own events.
    const { community, member, denied } = g.effectivePermissionsChanged;
    if (communityState.communities[community]?.myPseudonymKey !== member) return;
    // What we may no longer do, and why (never-hide), straight from the event.
    // Every denied map is the Host's whole snapshot (it serializes only the
    // denied actions), so it replaces the old map by reconcile: Solid merges a
    // plain object set at a path, which would keep a reason the Host dropped.
    if (denied) setCommunityState("communities", community, "denied", reconcile(denied));
    // Our rank may have moved, so the per-member moderation verdicts too.
    refreshCommunityMembers(community);
    readCommunityPermissions(community, false);
    return;
  }

  if ("governanceRebuilt" in g) {
    // A CRDT rebuild moves everything at once, so this is the one
    // governance event with no payload and a full re-read.
    const { community } = g.governanceRebuilt;
    readCommunityPermissions(community, true);
    // Also refresh members so the member list shows up
    commands
      .getCommunityMembers(community)
      .then((members) => {
        setCommunityMembers(community, members.map(transformMember));
      })
      .catch(() => {});
    void handleLoadExpressions(community);
    void handleLoadAutoModRules(community);
    if (communityState.activeCommunity === community && communityState.activeChannel) {
      void handleLoadChannelThreads(community, communityState.activeChannel);
    }
  }
}
