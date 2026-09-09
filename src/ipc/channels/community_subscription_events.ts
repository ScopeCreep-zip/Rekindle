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
import type { TypingContext } from "./chat_events";
import type { EventInfo } from "../commands/types";
import type { GameServer, Thread } from "../../stores/community.store";

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
  /** `community` is null for a device-wide announcement. */
  | { announcement: { community: string | null; body: string; timestamp: number } }
  /** A moderator's decision, gossiped to everyone. */
  | { raidAlert: { community: string; active: boolean } }
  /** **This peer's** own observation — see `raidAlert` for the decision. */
  | {
      raidDetected: {
        community: string;
        joinsInWindow: number;
        maxJoinsPerInterval: number;
        joinIntervalSeconds: number;
      };
    }
  | {
      autoModAlert: {
        community: string;
        channel: string;
        messageId: string;
        ruleName: string;
      };
    }
  | { channelLockdown: { community: string; locked: boolean } }
  | { bootstrapRequested: Record<string, unknown> }
  | { bootstrapReceived: Record<string, unknown> }
  | { syncRequested: Record<string, unknown> }
  | { syncReceived: { community: string; channel: string; messageCount: number } }
  | { auditChainBroken: { cursor: number } };

/** Cryptographic key events from the daemon vocabulary. */
export type CryptoEvent =
  | {
      mekRotated: {
        community: string;
        /** `null` for the community-wide key rather than a channel's. */
        channel: string | null;
        generation: number;
        rotatorPseudonym: string | null;
      };
    }
  | {
      mekRequested: {
        community: string;
        channel: string;
        neededGeneration: number;
        requesterPseudonym: string;
      };
    }
  | {
      mekTransferred: {
        community: string;
        channel: string | null;
        generation: number;
        senderPseudonym: string;
      };
    }
  | { adminKeypairGranted: { community: string } }
  | { slotKeypairGranted: { community: string; slotIndex: number; segmentIndex: number } }
  | { pqBundlePublished: { subkey: number; kind: string } };

// Community member presence is **not** in the union below: it is a
// `PresenceEvent`, which the backend routes to `presence-event`. See
// `subscribeCommunityPresenceEvents` in `presence-events.handlers.ts`.

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

/** Social events — reactions, pins, threads, scheduled events, servers. */
export type SocialEvent =
  | {
      reactionAdded: {
        community: string;
        channel: string;
        messageId: string;
        emoji: string;
        reactorPseudonym: string;
      };
    }
  | {
      reactionRemoved: {
        community: string;
        channel: string;
        messageId: string;
        emoji: string;
        reactorPseudonym: string;
      };
    }
  | {
      messagePinned: {
        community: string;
        channel: string;
        messageId: string;
        pinnedBy: string;
      };
    }
  | { messageUnpinned: { community: string; channel: string; messageId: string } }
  | { threadCreated: { community: string; thread: Thread } }
  | {
      threadMessagePosted: {
        community: string;
        threadId: string;
        messageId: string;
        senderPseudonym: string;
        timestamp: number;
        /** `null` from the gossip decoder, which sees only ciphertext. */
        body: string | null;
        replyToId: string | null;
      };
    }
  | { threadArchiveChanged: { community: string; threadId: string; archived: boolean } }
  | { eventCreated: { community: string; event: EventInfo } }
  | { eventUpdated: { community: string; event: EventInfo } }
  | { eventDeleted: { community: string; eventId: string } }
  | {
      eventRsvpChanged: {
        community: string;
        eventId: string;
        pseudonym: string;
        rsvpStatus: string;
      };
    }
  | {
      eventReminder: {
        community: string;
        eventId: string;
        title: string;
        minutesUntilStart: number;
      };
    }
  | { gameServerAdded: { community: string; server: GameServer } }
  | { gameServerRemoved: { community: string; serverId: string } };

/**
 * Channel-message lifecycle.
 *
 * Only the community-channel variants arrive here — the backend routes
 * the direct-conversation ones to `chat-event`, because those belong to
 * the chat windows rather than the community window.
 */
export type ChannelMessageEvent =
  | { new: Record<string, unknown> }
  | {
      edited: {
        community: string;
        channel: string;
        messageId: string;
        editedAt: number;
        body: string;
      };
    }
  | { deleted: { community: string; channel: string; messageId: string } };

export type CommunitySubscriptionEvent =
  | { membership: MembershipEvent }
  | { system: SystemEvent }
  | { governance: GovernanceEvent }
  | { social: SocialEvent }
  | { channelMessage: ChannelMessageEvent }
  | { crypto: CryptoEvent }
  /**
   * Channel typing. DM typing goes to `chat-event` — `TypingContext`
   * carries the distinction and the backend routes on it.
   */
  | {
      typing:
        | { started: { context: TypingContext; who: string } }
        | { stopped: { context: TypingContext; who: string } };
    }
  | { unreadChanged: Record<string, unknown> };

export type AnyCommunityEvent = CommunityEvent | CommunitySubscriptionEvent;

/** Whether this is the desktop's `{ type, data }` envelope. */
export function isLegacyCommunityEvent(
  event: AnyCommunityEvent,
): event is CommunityEvent {
  return "type" in event;
}
