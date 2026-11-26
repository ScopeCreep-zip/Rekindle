import type { Undecryptable } from "../commands/types";
import type { DeniedCapabilities, SkinApplied } from "../commands/dto";
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
import type { TypingContext, UnreadChanged } from "./chat_events";
import type { EventInfo, GameServer, Thread } from "../commands/types";

/** Why an inviter refused a join (`rekindle_types::join_refusal::JoinRefusal`). */
export type JoinRefusal = "INVITE_USED_UP" | "INVITE_NO_LONGER_VALID" | "INVITE_UNKNOWN" | "NOT_ADMITTED";

/** Membership events from the daemon vocabulary. */
export type MembershipEvent =
  /**
   * Our join was withdrawn after it returned: the inviter refused it on a
   * retried exchange and the Host forgot the community (plan E3.1/JN1).
   * `reasonText` is the Host's words for the user; render it as given.
   */
  | { joinWithdrawn: { community: string; reason: JoinRefusal; reasonText: string } }
  | { joinRequested: { community: string; pseudonym: string; displayName: string; hasInvite: boolean } }
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
  | { memberCardChanged: { community: string; pseudonym: string; displayName: string; customStatus?: string } }
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
  | { timedOut: { community: string; targetPseudonym: string; durationSeconds: number; reason: string | null } }
  | { timeoutRemoved: { community: string; targetPseudonym: string } }
  | { timeoutStatusChanged: { community: string; pseudonym: string; timeoutUntil: number | null } }
  | { rolesChanged: { community: string; pseudonym: string; roleIds: string[] } }
  | { onboardingCompleted: { community: string; pseudonym: string; roleIds: string[] } }
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

/** A role, as Tier 1's `RoleDisplay`. */
interface RoleDisplay {
  /** The full 16-byte role id, 32 hex characters. */
  id: string;
  name: string;
  color: number;
  /** The name's readable text colour per skin base (plan E7.3). */
  tagColours: { dark: number; light: number };
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
interface ChannelOverviewDisplay {
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
  /** A restricted channel whose roster we are not on (FX3): shown, closed. */
  locked?: boolean;
  /** The Host's words for why it is locked. */
  lockedReason?: string;
}

/** A channel category, as Tier 1's `CategoryDisplay`. */
interface CategoryDisplay {
  id: string;
  name: string;
  sortOrder: number;
}

/** Plate Gate (architecture §15) expansion segment, as Tier 1's `SegmentDescriptor`. */
interface SegmentDescriptor {
  segmentIndex: number;
  registryKey: string;
  anchor: string;
  slotRangeStart: number;
  slotRangeEnd: number;
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
  | { segmentsChanged: { community: string; segments: SegmentDescriptor[] } }
  | { bansChanged: { community: string } }
  | { opPublication: { community: string; op: string; state: OpPublicationState } }
  | { catchUp: { community: string; catchingUp: boolean; pending: PendingGovOp[] } }
  | { keyStatus: { community: string; status: KeyStatus } }
  | {
      inviteCreated: {
        community: string;
        inviteId: string;
        codeHash: string;
        createdBy: string;
        maxUses: number;
        uses: number;
        expiresAt: number | null;
        createdAt: number;
      };
    }
  | { inviteUsed: { community: string; codeHash: string; uses: number } }
  | { inviteRevoked: { community: string; codeHash: string } }
  | { inviteExpired: { community: string; codeHash: string } }
  | { channelPermissionsChanged: { community: string; channel: string } }
  | { effectivePermissionsChanged: { community: string; member: string; denied: DeniedCapabilities } }
  | { governanceRebuilt: { community: string } }
  | {
      governanceOpQuarantined: {
        community: string;
        op: string;
        author: string;
        reason: OpQuarantineReason;
        cause: string | null;
      };
    }
  | {
      /** A community joined from a checkpoint hint finished its backfill and
       *  the full history's state at the hint's cut is not the hinted
       *  snapshot (plan E3.2 S7c.8, threat T8); the community stays on the
       *  full history. Height and digest are the hint's checkpoint. */
      checkpointSnapshotMismatch: { community: string; height: number; digest: string };
    }
  | {
      /** A successor community both sides link (ADR 0011, plan E3.2 S7s.3e):
       *  an applied successor op here names `newAnchor`, and its verified
       *  genesis cert names this community as predecessor. Announced once
       *  per op; two valid successors are both offered. `signer` and
       *  `creators` are pseudonyms, hex. */
      communitySuccessorOffered: {
        community: string;
        newAnchor: string;
        signer: string;
        reason: "recreate" | "transfer";
        creators: string[];
      };
    }
  | {
      /** A transfer's link request asks us (lead decision (46)):
       *  `recipient`, the offer's recipient, asks this creator to link into
       *  its successor `newAnchor`. Announced once per request while it is
       *  unanswered and its offer stands; answered with
       *  `AnswerSuccessorLink`. `request` and `recipient` are hex. */
      successorLinkRequested: {
        community: string;
        request: string;
        recipient: string;
        newAnchor: string;
      };
    };

/** Why the backend set a governance op aside, never applied
 *  (`GovernanceEvent::GovernanceOpQuarantined`, plan E3.2 S7c.5). The two
 *  `unknownOp*` reasons quarantine the whole community (D15): a member
 *  signed an op this version cannot read, likely a newer version's. */
export type OpQuarantineReason =
  | "afterAuthorRemoval"
  | "twoCheckpointEndorsementsInOneRound"
  | "recordClaimed"
  | "authorityQuarantined"
  | "deviceKeyRetired"
  | "deviceKeyInert"
  | "deviceKeyOverCap"
  | "deviceKeyNeedsReadmit"
  | "deviceKeyRecordTaken"
  | "checkpointEquivocation"
  | "unknownOpKind"
  | "unknownOpVersion";

/** Where one of our governance ops stands, decided by the backend
 *  (`GovernanceEvent::OpPublication`): pending while queued, landed once on
 *  our member record and applied, failed with the community's reason. */
export type OpPublicationState =
  | { kind: "pending" }
  | { kind: "landed" }
  | { kind: "failed"; reason: string };

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
      typing:
        // `name`: the typist's name as the Host states it, never empty (card name, else short fingerprint).
        | { started: { context: TypingContext; who: string; name: string } }
        | { stopped: { context: TypingContext; who: string } };
    }
  | { unreadChanged: UnreadChanged };

export type AnyCommunityEvent = CommunityEvent | CommunitySubscriptionEvent;

/** Whether this is the desktop's `{ type, data }` envelope. */
export function isLegacyCommunityEvent(
  event: AnyCommunityEvent,
): event is CommunityEvent {
  return "type" in event;
}

/** The unlock barrier's view (plan E3.2 S7d-keys.8, `display_catch_up.rs`): sent in full on every change by
 *  `GovernanceEvent::CatchUp`, and on re-read as `CommunityDetail.catch_up`. Cleared by `catchingUp: false, pending: []`. */
export interface CatchUpView {
  catchingUp: boolean;
  pending: PendingGovOp[];
}

/** One governance write held by the unlock barrier. */
export interface PendingGovOp {
  opId: string;
  kind: PendingGovKind;
  target: { member: string | null; channel: string | null; role: string | null };
  /** The Host's words, e.g. "Kick Bea". */
  label: string;
  state: "catchingUp" | "signing";
}

/** One per `GovernanceEntry` arm (`PendingGovKind`). */
type PendingGovKind =
  | "channelCreated" | "channelEdited" | "channelDeleted" | "roleEdited" | "rolesChanged" | "roleDeleted"
  | "ban" | "unban" | "join" | "memberRecordMoved" | "leave" | "kick" | "approveMember" | "rejectMember"
  | "timeout" | "removeTimeout" | "communityEdited" | "notificationDefault" | "keyRotation"
  | "categoryCreated" | "categoryEdited" | "categoryDeleted" | "permissionsChanged"
  | "threadCreated" | "threadArchived" | "threadUnarchived" | "eventCreated" | "eventArchived" | "eventRsvp"
  | "expressionAdded" | "expressionRemoved" | "modReport" | "onboardingConfig" | "welcomeScreen"
  | "adminDelete" | "segmentAdded" | "autoModRule" | "inviteCreated" | "inviteRevoked" | "attachmentPinned"
  | "communityPolicy" | "checkpoint" | "successor" | "deviceKey" | "deviceRetired" | "deviceReadmit"
  | "successorLinkRequest" | "successorLink";

/** Our community key against governance's generation, as the Host states it
 *  (`rekindle_types::dto::community::KeyStatus`; `CommunityView::key_status`,
 *  `GovernanceEvent::KeyStatus`): `pending` while it is missing or behind. */
export interface KeyStatus {
  state: "current" | "pending";
  have: number | null;
  need: number | null;
  sinceMs: number | null;
}
