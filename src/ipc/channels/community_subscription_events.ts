import type { SkinApplied } from "../commands/dto";
import type { Undecryptable } from "../commands/types";

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

import type { EventInfo, GameServer, Thread } from "../commands/types";
import type { TypingContext, UnreadChanged } from "./chat_events";
import type { CommunityEvent } from "./community_events";
import type { GovernanceEvent } from "./community_governance_events";

/** Why an inviter refused a join (`rekindle_types::join_refusal::JoinRefusal`). */
type JoinRefusal = "INVITE_USED_UP" | "INVITE_NO_LONGER_VALID" | "INVITE_UNKNOWN" | "NOT_ADMITTED";

/** Membership events from the daemon vocabulary. */
type MembershipEvent =
  /**
   * Our join was withdrawn after it returned: the inviter refused it on a
   * retried exchange and the Host forgot the community (plan E3.1/JN1).
   * `reasonText` is the Host's words for the user; render it as given.
   */
  | { joinWithdrawn: { community: string; reason: JoinRefusal; reasonText: string } }
  | {
      joinRequested: {
        community: string;
        pseudonym: string;
        displayName: string;
        hasInvite: boolean;
      };
    }
  | { joinProgress: { community: string; stage: string; status: string } }
  | {
      joined: {
        community: string;
        pseudonym: string;
        /** The verified card's name; null until the card is read (plan E3.2 F9-2). */
        displayName: string | null;
        roleIds: string[];
      };
    }
  /** The name on a member's verified card, first read or changed (plan E3.2 F9-2). */
  | {
      memberCardChanged: {
        community: string;
        pseudonym: string;
        displayName: string;
        customStatus?: string;
      };
    }
  | { left: { community: string; pseudonym: string } }
  /** We no longer hold the community: our leave, withdrawn join or Forget (plan E3.2). Device-scoped; a repeat is a no-op. */
  | { forgotten: { community: string } }
  | { removed: { community: string; pseudonym: string } }
  | {
      kicked: {
        community: string;
        targetPseudonym: string;
        removedBy: string | null;
        removedByName: string | null;
        /** The removal op's reason; null from gossip or when none was given. */
        reason: string | null;
      };
    }
  | {
      banned: {
        community: string;
        targetPseudonym: string;
        removedBy: string | null;
        removedByName: string | null;
        /** The removal op's reason; null from gossip or when none was given. */
        reason: string | null;
      };
    }
  | { unbanned: { community: string; targetPseudonym: string } }
  | {
      timedOut: {
        community: string;
        targetPseudonym: string;
        durationSeconds: number;
        reason: string | null;
      };
    }
  | { timeoutRemoved: { community: string; targetPseudonym: string } }
  | { timeoutStatusChanged: { community: string; pseudonym: string; timeoutUntil: number | null } }
  | { rolesChanged: { community: string; pseudonym: string; roleIds: string[] } }
  | { onboardingCompleted: { community: string; pseudonym: string; roleIds: string[] } }
  | {
      onboardingAnswersSubmitted: {
        community: string;
        senderPseudonym: string;
        answerCount: number;
      };
    }
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
type SystemEvent =
  /** **This peer's** own observation of the join rate. */
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
  | { bootstrapRequested: Record<string, unknown> }
  | { bootstrapReceived: Record<string, unknown> }
  | { auditChainBroken: { cursor: number } }
  | { skinChanged: { skin: SkinApplied } };

/** Cryptographic key events from the daemon vocabulary. */
type CryptoEvent =
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
  | { prekeyRecordPublished: { head: boolean; slots: number[] } };

// Community member presence is **not** in the union below: it is a
// `PresenceEvent`, which the backend routes to `presence-event`. See
// `subscribeCommunityPresenceEvents` in `presence-events.handlers.ts`.

/** Social events — reactions, pins, threads, scheduled events, servers. */
type SocialEvent =
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
  | { pollChanged: { community: string; channel: string; pollId: string } }
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
  | {
      new: {
        community: string;
        channel: string;
        messageId: string;
        senderPseudonym: string;
        sequence: number;
        timestamp: number;
        /** `null` when the channel's text key was not cached at emission. */
        body: string | null;
        /** The message id of the parent this one replies to. */
        replyToMessageId: string | null;
        /** The original author's pseudonym when the post is a forward (the
         *  value history serves as `forwardedFromAuthor`); `null` otherwise. */
        forwardedFromAuthor: string | null;
        /** That author as the community names them: the display form a
         *  sender header shows (never the raw key); `null` otherwise. */
        forwardedFromName: string | null;
        /** The D26 gate hid it (an automod `BlurContent` match). */
        automodBlurred: boolean;
        /** Stored without its body, and why (`body` is `null`); a held post never has it. */
        undecryptable?: Undecryptable;
      };
    }
  | {
      edited: {
        community: string;
        channel: string;
        messageId: string;
        editedAt: number;
        body: string;
      };
    }
  /** Deleted for members (plan E3.3 E4); `removedBy` is the moderator
   *  when a moderator removed it (M4), `null` for its author's delete. */
  | { deleted: { community: string; channel: string; messageId: string; removedBy: string | null } }
  /** Our own post settled at consensus (`sent`), was superseded or its key
   *  never came (`failed`, with the `reason` to show), waits for the
   *  community key (`awaiting_key`) or was released to its write (`queued`). */
  | {
      channelSendStatus: {
        community: string;
        channel: string;
        messageId: string;
        state: "awaiting_key" | "queued" | "sent" | "delivered" | "read" | "failed";
        reason?: string;
      };
    }
  /** The author's outbox reaches `throughSeq` and no read reached it (plan
   *  E3.3, D23): the Host stored a gap row in `channel`; shown as the
   *  history would show it. */
  | {
      channelHistoryGap: {
        community: string;
        channel: string;
        senderPseudonym: string;
        throughSeq: number;
      };
    }
  /** A read reached the head: the author's gap rows through `throughSeq`
   *  were removed in every channel of `community`. */
  | {
      channelHistoryGapClosed: {
        community: string;
        senderPseudonym: string;
        throughSeq: number;
      };
    }
  /** An attachment finished downloading to the place the user chose. */
  | {
      attachmentDownloaded: {
        community: string;
        channel: string;
        attachmentId: string;
      };
    }
  /** A custom emoji's or sticker's bytes are in the chunk cache. */
  | { expressionAssetReady: { community: string; expressionId: string } };

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
      typing: // `name`: the typist's name as the Host states it, never empty (card name, else short fingerprint).
        | { started: { context: TypingContext; who: string; name: string } }
        | { stopped: { context: TypingContext; who: string } };
    }
  | { unreadChanged: UnreadChanged };

export type AnyCommunityEvent = CommunityEvent | CommunitySubscriptionEvent;

/** Whether this is the desktop's `{ type, data }` envelope. */
export function isLegacyCommunityEvent(event: AnyCommunityEvent): event is CommunityEvent {
  return "type" in event;
}
