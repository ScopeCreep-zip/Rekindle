import { batch } from "solid-js";
import { produce, reconcile } from "solid-js/store";
import { handleLoadChannelThreads } from "../../actions/community/events_threads";
import {
  handleLoadAutoModRules,
  handleLoadExpressions,
  handleResolveCommunityImageDataUrls,
} from "../../actions/community/lifecycle";
import type { GovernanceEvent } from "../../ipc/channels/community_governance_events";
import { commands } from "../../ipc/commands";
import {
  type Community,
  communityState,
  setCommunityMembers,
  setCommunityState,
} from "../../stores/community.store";
import { applySuccessorEvent } from "../../stores/successor.store";
import { transformChannel, transformMember } from "../../utils/transformers";
import { withCommunity } from "./community_reread";
import { refreshCommunityMembers } from "./members_refresh";

/// Governance slice of the community event dispatcher: roles, channels,
/// segments, keys and our own ops, each applied under the row it names.

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
      if (
        !detail ||
        communityState.communities[community] !== row ||
        permissionReads.get(row) !== read
      )
        return;
      permissionReads.delete(row);
      batch(() => {
        setCommunityState("communities", community, "myPermissions", detail.myPermissions);
        setCommunityState("communities", community, "denied", reconcile(detail.denied ?? {}));
        setCommunityState(
          "communities",
          community,
          "myChannelPermissions",
          detail.myChannelPermissions
        );
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
            "communities",
            community,
            "channels",
            (c) => c.id === ch.id,
            "denied",
            reconcile(ch.denied ?? {})
          );
        }
      });
    })
    .catch(() => {});
}

export function applyGovernanceEvent(g: GovernanceEvent): void {
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
    if (communityState.communities[community])
      setCommunityState("communities", community, "keyStatus", status);
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
        }))
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
      }))
    );
    setCommunityState(
      "communities",
      community,
      "categories",
      categories.map((cat) => ({
        id: cat.id,
        name: cat.name,
        sortOrder: cat.sortOrder,
      }))
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
      void commands
        .listCommunityInvites(community)
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
      setCommunityState(
        "opPublications",
        community,
        produce((ops) => {
          delete ops[op];
        })
      );
    }
    return;
  }

  if ("inviteRevoked" in g || "inviteExpired" in g) {
    const { community, codeHash } = "inviteRevoked" in g ? g.inviteRevoked : g.inviteExpired;
    setCommunityState("communityInvites", community, (prev) =>
      (prev ?? []).filter((inv) => inv.codeHash !== codeHash)
    );
    return;
  }

  if ("inviteUsed" in g) {
    const { community, codeHash, uses } = g.inviteUsed;
    setCommunityState("communityInvites", community, (prev) =>
      (prev ?? []).map((inv) => (inv.codeHash === codeHash ? { ...inv, uses } : inv))
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
          // The overwrite moves what we may do in that channel: its served
          // denied map and lock replace the ones we held (reconcile: a held
          // capability is absent from the Host's map), so New post, the reply
          // box and the other controls show the Host's reasons at once.
          const served = detail.channels.find((c: { id: string }) => c.id === channel);
          const idx =
            communityState.communities[community]?.channels.findIndex((c) => c.id === channel) ??
            -1;
          if (served && idx >= 0) {
            const view = transformChannel(served);
            setCommunityState(
              "communities",
              community,
              "channels",
              idx,
              "denied",
              reconcile(view.denied)
            );
            setCommunityState("communities", community, "channels", idx, "locked", view.locked);
            setCommunityState(
              "communities",
              community,
              "channels",
              idx,
              "lockedReason",
              view.lockedReason
            );
          }
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
