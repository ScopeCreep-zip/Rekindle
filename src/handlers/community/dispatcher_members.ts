import type { CommunityEvent } from "../../ipc/channels";
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
  } else if (event.type === "memberJoined") {
    const { communityId, pseudonymKey, displayName, roleIds } = event.data;
    const community = communityState.communities[communityId];
    if (community) {
      const exists = community.members.some((m) => m.pseudonymKey === pseudonymKey);
      if (!exists) {
        setCommunityState("communities", communityId, "members", (prev) => [
          ...prev,
          transformMember({ pseudonymKey, displayName, roleIds, displayRole: "", status: "online", timeoutUntil: null }),
        ]);
      }
    }
    return true;
  } else if (event.type === "memberRemoved") {
    const { communityId, pseudonymKey } = event.data;
    setCommunityState("communities", communityId, "members", (prev) =>
      prev.filter((m) => m.pseudonymKey !== pseudonymKey),
    );
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
  } else if (event.type === "memberRolesChanged") {
    const { communityId, pseudonymKey, roleIds: newRoleIds } = event.data;
    const community = communityState.communities[communityId];
    if (community) {
      const idx = community.members.findIndex((m) => m.pseudonymKey === pseudonymKey);
      if (idx >= 0) {
        setCommunityState("communities", communityId, "members", idx, "roleIds", newRoleIds);
      }
      if (pseudonymKey === community.myPseudonymKey) {
        setCommunityState("communities", communityId, "myRoleIds", newRoleIds);
      }
    }
    return true;
  } else if (event.type === "memberTimedOut") {
    const { communityId, pseudonymKey, timeoutUntil } = event.data;
    const community = communityState.communities[communityId];
    if (community) {
      const idx = community.members.findIndex((m) => m.pseudonymKey === pseudonymKey);
      if (idx >= 0) {
        setCommunityState("communities", communityId, "members", idx, "timeoutUntil", timeoutUntil);
      }
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
  } else if (event.type === "membersRefreshed") {
    const { communityId } = event.data;
    commands.getCommunityMembers(communityId).then((members) => {
      setCommunityState("communities", communityId, "members", members.map(transformMember));
    }).catch(() => {});
    return true;
  } else if (event.type === "kicked") {
    const { communityId } = event.data;
    setCommunityState("communities", communityId, undefined!);
    if (communityState.activeCommunity === communityId) {
      setCommunityState("activeCommunity", null);
      setCommunityState("activeChannel", null);
    }
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
  } else if (event.type === "memberDiscovered") {
    const { communityId, pseudonymKey, displayName } = event.data;
    const community = communityState.communities[communityId];
    if (community) {
      const exists = community.members.some((m) => m.pseudonymKey === pseudonymKey);
      if (!exists) {
        setCommunityState("communities", communityId, "members", (prev) => [
          ...prev,
          transformMember({ pseudonymKey, displayName, roleIds: [0, 1], displayRole: "", status: "online", timeoutUntil: null }),
        ]);
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
  } else if (event.type === "onboardingComplete") {
    const { communityId, pseudonymKey, roleIds } = event.data;
    const community = communityState.communities[communityId];
    const members = community?.members ?? [];
    const idx = members.findIndex((m) => m.pseudonymKey === pseudonymKey);
    if (idx >= 0) {
      setCommunityState("communities", communityId, "members", idx, "roleIds", roleIds);
    }
    if (community?.myPseudonymKey === pseudonymKey) {
      setCommunityState("communities", communityId, "onboardingComplete", true);
    }
    return true;
  }
  return false;
}
