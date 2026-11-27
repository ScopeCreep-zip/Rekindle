import type { DeniedCapabilities } from "../commands/dto_capabilities";

// The governance vocabulary of the community subscription: what the merged
// governance state changed (roles, channels, segments, keys, our own ops)
// and the unlock barrier's view of the writes it holds.

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
  | {
      effectivePermissionsChanged: {
        community: string;
        member: string;
        denied: DeniedCapabilities;
      };
    }
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

/** The unlock barrier's view (plan E3.2 S7d-keys.8, `display_catch_up.rs`): sent in full on every change by
 *  `GovernanceEvent::CatchUp`, and on re-read as `CommunityDetail.catch_up`. Cleared by `catchingUp: false, pending: []`. */
export interface CatchUpView {
  catchingUp: boolean;
  pending: PendingGovOp[];
}

/** One governance write held by the unlock barrier. */
interface PendingGovOp {
  opId: string;
  kind: PendingGovKind;
  target: { member: string | null; channel: string | null; role: string | null };
  /** The Host's words, e.g. "Kick Bea". */
  label: string;
  state: "catchingUp" | "signing";
}

/** One per `GovernanceEntry` arm (`PendingGovKind`). */
type PendingGovKind =
  | "channelCreated"
  | "channelEdited"
  | "channelDeleted"
  | "roleEdited"
  | "rolesChanged"
  | "roleDeleted"
  | "ban"
  | "unban"
  | "join"
  | "memberRecordMoved"
  | "leave"
  | "kick"
  | "approveMember"
  | "rejectMember"
  | "timeout"
  | "removeTimeout"
  | "communityEdited"
  | "notificationDefault"
  | "keyRotation"
  | "categoryCreated"
  | "categoryEdited"
  | "categoryDeleted"
  | "permissionsChanged"
  | "threadCreated"
  | "threadArchived"
  | "threadUnarchived"
  | "eventCreated"
  | "eventArchived"
  | "eventRsvp"
  | "expressionAdded"
  | "expressionRemoved"
  | "modReport"
  | "onboardingConfig"
  | "welcomeScreen"
  | "adminDelete"
  | "segmentAdded"
  | "autoModRule"
  | "inviteCreated"
  | "inviteRevoked"
  | "attachmentPinned"
  | "communityPolicy"
  | "checkpoint"
  | "successor"
  | "deviceKey"
  | "deviceRetired"
  | "deviceReadmit"
  | "deviceReEnrol"
  | "successorLinkRequest"
  | "successorLink";

/** Our community key against governance's generation, as the Host states it
 *  (`rekindle_types::dto::community::KeyStatus`; `CommunityView::key_status`,
 *  `GovernanceEvent::KeyStatus`): `pending` while it is missing or behind. */
export interface KeyStatus {
  state: "current" | "pending";
  have: number | null;
  need: number | null;
  sinceMs: number | null;
}
