import type { CommunityEvent } from "../../ipc/channels";
import type { CommunitySubscriptionEvent } from "../../ipc/channels/community_events";
import { applyJoinProgress, type JoinStageStatus } from "../../stores/join.store";
import { transformCommunityDetail } from "../../utils/transformers";
import { handleResolveCommunityImageDataUrls } from "../../actions/community/lifecycle";
import { setCommunityState, communityState } from "../../stores/community.store";
import { commands } from "../../ipc/commands";
import { addToast } from "../../stores/toast.store";
import { transformChannel, transformMember } from "../../utils/transformers";
import { handleLoadExpressions, handleLoadAutoModRules } from "../../actions/community/lifecycle";
import { handleLoadChannelThreads } from "../../actions/community/events_threads";

/// Membership / governance / moderation-alert slice of the community
/// event dispatcher. Returns `true` when the event was consumed.
export function reduceMembership(event: CommunityEvent): boolean {
  if (event.type === "expressionAssetReady") {
    // Architecture §18.4 — eager-fetch landed for this expression;
    // refresh the community's expression list so the picker re-renders
    // with the resolved inline_data_base64 instead of `:emojiname:`.
    void handleLoadExpressions(event.data.communityId);
    return true;
  } else if (event.type === "raidDetected") {
    // Architecture §20.6 — backend's per-community sliding window
    // tripped the policy threshold; surface a moderator banner via
    // the toast layer. The user can then take the spec-listed
    // actions (pause invites, ban floods, raise verification).
    const { joinsInWindow, maxJoinsPerInterval, joinIntervalSeconds } = event.data;
    addToast(
      `Raid detected: ${joinsInWindow} joins in the last ${joinIntervalSeconds}s ` +
        `(threshold ${maxJoinsPerInterval}). Consider pausing invites.`,
      "error",
    );
    return true;
  } else if (event.type === "rolesChanged") {
    const { communityId, roles } = event.data;
    if (communityState.communities[communityId]) {
      setCommunityState("communities", communityId, "roles", roles);
    }
    return true;
  } else if (event.type === "channelOverwriteChanged") {
    const { communityId } = event.data;
    if (communityState.communities[communityId]) {
      commands.getCommunityDetails().then((details) => {
        const detail = details.find((d: { id: string }) => d.id === communityId);
        if (detail) {
          setCommunityState("communities", communityId, "roles", detail.roles);
        }
      }).catch(() => {});
    }
    return true;
  } else if (event.type === "governanceUpdated") {
    // CRDT governance state changed — refresh community details + members
    const { communityId } = event.data;
    commands.getCommunityDetails().then((details) => {
      const detail = details.find((c: { id: string }) => c.id === communityId);
      if (detail) {
        setCommunityState("communities", communityId, "name", detail.name);
        setCommunityState("communities", communityId, "description", detail.description ?? null);
        setCommunityState("communities", communityId, "roles", detail.roles ?? []);
        setCommunityState("communities", communityId, "channels",
          detail.channels.map(transformChannel));
        setCommunityState("communities", communityId, "categories", detail.categories ?? []);
        setCommunityState("communities", communityId, "myRoleIds", detail.myRoleIds ?? [0]);
        setCommunityState("communities", communityId, "mekGeneration", detail.mekGeneration ?? 0);
      }
    }).catch(() => {});
    // Also refresh members so the member list shows up
    commands.getCommunityMembers(communityId).then((members) => {
      setCommunityState("communities", communityId, "members", members.map(transformMember));
    }).catch(() => {});
    void handleLoadExpressions(communityId);
    void handleLoadAutoModRules(communityId);
    if (communityState.activeCommunity === communityId && communityState.activeChannel) {
      void handleLoadChannelThreads(communityId, communityState.activeChannel);
    }
    return true;
  } else if (event.type === "autoModAlert") {
    addToast(`AutoMod alert: ${event.data.ruleName}`, "info");
    return true;
  } else if (event.type === "memberPresenceChanged") {
    const { communityId, pseudonymKey, status } = event.data;
    const community = communityState.communities[communityId];
    if (community) {
      const idx = community.members.findIndex((m) => m.pseudonymKey === pseudonymKey);
      if (idx >= 0) {
        setCommunityState("communities", communityId, "members", idx, "status", status);
        // A member that just went offline is no longer "in" any channel —
        // clear their location so the channel presence badge stops counting
        // them without waiting for the next full members re-fetch.
        if (status === "offline") {
          setCommunityState("communities", communityId, "members", idx, "location", null);
        }
        const gameInfo = event.data.gameName
          ? {
              gameName: event.data.gameName,
              gameId: event.data.gameId ?? null,
              startedAt: event.data.elapsedSeconds ?? null,
              serverAddress: event.data.serverAddress ?? null,
            }
          : null;
        setCommunityState("communities", communityId, "members", idx, "gameInfo", gameInfo);
      }
    }
    return true;
  } else if (event.type === "raidAlert") {
    // Architecture §17.4 — raid alert lives in store; CommunityWindow
    // renders a banner overlay (`role="alert"`) for higher visibility
    // than a transient toast. The flag persists until the backend
    // emits `active: false` (or the user clears it client-side via
    // `dismissRaidAlertLocal`).
    const { communityId, active } = event.data;
    setCommunityState("communities", communityId, "raidAlertActive", active);
    return true;
  } else if (event.type === "channelLockdown") {
    const { communityId, locked } = event.data;
    const name = communityState.communities[communityId]?.name ?? communityId;
    addToast(locked ? `Channels locked in ${name}` : `Channel lockdown lifted in ${name}`, "info");
    return true;
  }
  return false;
}

/// Membership events on the daemon vocabulary. Same store effects as
/// the `{ type, data }` cases they replaced; only the shape changed.
export function reduceSubscriptionMembership(
  event: CommunitySubscriptionEvent,
): void {
  if ("system" in event) {
    const sys = event.system;
    if ("kicked" in sys) {
      // We were removed from the community.
      const communityId = sys.kicked.community;
      setCommunityState("communities", communityId, undefined!);
      if (communityState.activeCommunity === communityId) {
        setCommunityState("activeCommunity", null);
        setCommunityState("activeChannel", null);
      }
    }
    return;
  }
  if (!("membership" in event)) return;
  const m = event.membership;

  if ("joined" in m) {
    const { community, pseudonym, displayName, roleIds } = m.joined;
    const c = communityState.communities[community];
    if (c && !c.members.some((x) => x.pseudonymKey === pseudonym)) {
      setCommunityState("communities", community, "members", (prev) => [
        ...prev,
        transformMember({
          pseudonymKey: pseudonym,
          displayName,
          roleIds,
          displayRole: "",
          status: "online",
          timeoutUntil: null,
        }),
      ]);
    }
    return;
  }

  if ("removed" in m) {
    const { community, pseudonym } = m.removed;
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
        setCommunityState("communities", community, "members", members.map(transformMember));
      })
      .catch(() => {});
    return;
  }

  if ("memberDiscovered" in m) {
    const { community, pseudonym, displayName } = m.memberDiscovered;
    const c = communityState.communities[community];
    if (c && !c.members.some((x) => x.pseudonymKey === pseudonym)) {
      setCommunityState("communities", community, "members", (prev) => [
        ...prev,
        transformMember({
          pseudonymKey: pseudonym,
          displayName,
          roleIds: [0, 1],
          displayRole: "",
          status: "online",
          timeoutUntil: null,
        }),
      ]);
    }
    return;
  }

  if ("onboardingCompleted" in m) {
    const { community, pseudonym, roleIds } = m.onboardingCompleted;
    const c = communityState.communities[community];
    const idx = (c?.members ?? []).findIndex((x) => x.pseudonymKey === pseudonym);
    if (idx >= 0) {
      setCommunityState("communities", community, "members", idx, "roleIds", roleIds);
    }
    if (c?.myPseudonymKey === pseudonym) {
      setCommunityState("communities", community, "onboardingComplete", true);
    }
    return;
  }

  if ("joinAccepted" in m) {
    // Architecture §7.4 — peer accepted our join request and the MEK
    // has landed. Refresh the community detail so the new generation,
    // registry slot and governance state propagate into the store.
    const { community } = m.joinAccepted;
    addToast("Joined community — encryption keys received", "success");
    void commands.getCommunityDetails().then((details) => {
      const detail = details.find((d) => d.id === community);
      if (detail) {
        setCommunityState("communities", community, transformCommunityDetail(detail));
        void handleResolveCommunityImageDataUrls(community);
      }
    });
    return;
  }

  if ("joinProgress" in m) {
    // Pure display: mirror the phase into the join store so the
    // stepper re-renders. No timeout logic — the backend gate owns
    // each phase's budget.
    const { stage, status } = m.joinProgress;
    applyJoinProgress(stage, status as JoinStageStatus);
    return;
  }

  if ("joinRejected" in m) {
    addToast(`Join rejected: ${m.joinRejected.reason}`, "error");
  }
}
