/**
 * The daemon-vocabulary half of the `community-event` channel.
 *
 * Split out of `community_events.ts` when that file hit the 600-line
 * ceiling. The seam is the migration boundary itself: this file holds
 * the Tier 1 families the CLI also consumes, `community_events.ts`
 * holds the desktop's remaining `{ type, data }` envelope, and
 * `isLegacyCommunityEvent` (defined here, since it discriminates
 * between the two) tells them apart.
 */

import type { CommunityEvent } from "./community_events";

/** Membership events from the daemon vocabulary. */
export type MembershipEvent =
  | { joinRequested: { community: string; pseudonym: string; displayName: string; hasInvite: boolean } }
  | { joinAccepted: { community: string; mekGeneration: number | null; slotIndex: number | null } }
  | { joinRejected: { community: string; reason: string } }
  | { joinProgress: { community: string; stage: string; status: string } }
  | { joined: { community: string; pseudonym: string; displayName: string; roleIds: number[] } }
  | { left: { community: string; pseudonym: string } }
  | { removed: { community: string; pseudonym: string } }
  | { kicked: { community: string; targetPseudonym: string } }
  | { banned: { community: string; targetPseudonym: string } }
  | { unbanned: { community: string; targetPseudonym: string } }
  | { timedOut: { community: string; targetPseudonym: string; durationSeconds: number; reason: string | null } }
  | { timeoutRemoved: { community: string; targetPseudonym: string } }
  | { timeoutStatusChanged: { community: string; pseudonym: string; timeoutUntil: number | null } }
  | { rolesChanged: { community: string; pseudonym: string; roleIds: number[] } }
  | { onboardingCompleted: { community: string; pseudonym: string; roleIds: number[] } }
  | { onboardingAnswersSubmitted: { community: string; senderPseudonym: string; answerCount: number } }
  | { membersRefreshed: { community: string } }
  | {
      /**
       * Found by scanning the registry rather than announced by gossip.
       * A frontend adds the row without the join chrome.
       */
      memberDiscovered: {
        community: string;
        pseudonym: string;
        displayName: string;
        subkeyIndex: number;
      };
    };

/** System-level signals from the daemon vocabulary. */
export type SystemEvent =
  | { kicked: { community: string } }
  | { announcement: Record<string, unknown> }
  | { raidAlert: Record<string, unknown> }
  | { channelLockdown: Record<string, unknown> }
  | { bootstrapRequested: Record<string, unknown> }
  | { bootstrapReceived: Record<string, unknown> }
  | { syncRequested: Record<string, unknown> }
  | { syncReceived: Record<string, unknown> };

/** A role, as Tier 1's `RoleDisplay`. */
export interface RoleDisplay {
  id: number;
  name: string;
  color: number;
  /**
   * Permission bitmask. A plain JSON number, safe while
   * `permissions::ALL` stays under 2^53 — the Rust side asserts that.
   */
  permissions: number;
  position: number;
  hoist: boolean;
  mentionable: boolean;
  selfAssignable: boolean;
  exclusionGroup: string | null;
}

/** A channel, as Tier 1's `ChannelOverviewDisplay`. */
export interface ChannelOverviewDisplay {
  id: string;
  name: string;
  /** `"text"` | `"voice"` | `"announcement"`. Called `type` in the store. */
  kind: string;
  categoryId: string | null;
  topic: string;
  mekGeneration: number;
  logKey: string | null;
  sortOrder: number;
  slowmodeSeconds: number | null;
}

/** A channel category, as Tier 1's `CategoryDisplay`. */
export interface CategoryDisplay {
  id: string;
  name: string;
  sortOrder: number;
}

/**
 * Governance events from the daemon vocabulary.
 *
 * These carry the new state inline rather than telling the client to
 * re-read it. `governanceRebuilt` is the exception: a CRDT rebuild
 * moves channels, roles, members and permissions together, so there is
 * no single payload and re-reading is correct.
 */
export type GovernanceEvent =
  | {
      metadataChanged: {
        community: string;
        name: string | null;
        description: string | null;
        iconHash: string | null;
        bannerHash: string | null;
      };
    }
  | {
      channelsChanged: {
        community: string;
        channels: ChannelOverviewDisplay[];
        categories: CategoryDisplay[];
      };
    }
  | { rolesChanged: { community: string; roles: RoleDisplay[] } }
  | { bansChanged: { community: string } }
  | {
      inviteCreated: {
        community: string;
        codeHash: string;
        createdBy: string;
        maxUses: number | null;
        uses: number;
        expiresAt: number | null;
        createdAt: number;
      };
    }
  | { inviteUsed: { community: string; codeHash: string; uses: number } }
  | { inviteRevoked: { community: string; codeHash: string } }
  | { channelPermissionsChanged: { community: string; channel: string } }
  | {
      governanceSubkeyUpdated: {
        community: string;
        subkeyIndex: number;
        lamportTs: number;
      };
    }
  | { governanceRebuilt: { community: string } };

/** The daemon-vocabulary families delivered on this channel. */
export type CommunitySubscriptionEvent =
  | { membership: MembershipEvent }
  | { system: SystemEvent }
  | { governance: GovernanceEvent }
  | { crypto: Record<string, unknown> }
  | { unreadChanged: Record<string, unknown> };

export type AnyCommunityEvent = CommunityEvent | CommunitySubscriptionEvent;

/** Whether this is the desktop's `{ type, data }` envelope. */
export function isLegacyCommunityEvent(
  event: AnyCommunityEvent,
): event is CommunityEvent {
  return "type" in event;
}
