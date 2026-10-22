// Backend payload shapes — DTOs, not store state.
//
// This file was `src/stores/types.ts`, and every export in it is a wire
// type the Rust side produces. That put four of them on the wrong side
// of the `ipc-is-leaf` boundary: `src/ipc/commands/system.ts` declares
// the commands that *return* `OnboardingConfig`, `WelcomeScreen`,
// `OnboardingAnswer` and `GossipDiagnostics`, so it had to reach up
// into `stores/` to name its own return types.
//
// `LifecycleState` moved here from `stores/lifecycle.store.ts` for the
// same reason: it is the FSM state `setup.rs` emits, and both
// `lifecycleCurrent()` and the channel subscription need to name it.

import type { SendState } from "./types";

export interface GameInfo {
  gameName: string;
  gameId: number | null;
  startedAt: number | null;
  serverAddress: string | null;
}

export interface AuditLogEntryDto {
  action: string;
  actorPseudonym: string;
  target: string | null;
  details: string | null;
  timestamp: number;
}

/** A live community invite (`rekindle_types::dto::community::CommunityInviteView`). */
export interface InviteDto {
  /** What a revoke names. */
  inviteId: string;
  codeHash: string;
  createdBy: string;
  maxUses: number | null;
  /** Counted only by the minter. */
  uses: number;
  /** Unix seconds. */
  expiresAt: number | null;
  /** Milliseconds; null for another member's invite. */
  createdAt: number | null;
  /** The invite link, only for invites this identity minted. */
  url: string | null;
}

// ── Friends and direct messages (plan E2.1; `rekindle_types::dto::{friends, dm}`) ──

/** One direct message, as a chat window shows it. */
export interface DmMessage {
  id: number;
  /** Its client id, which a reply names; null for a row from before E2.3. */
  messageId: string | null;
  senderId: string;
  body: string;
  timestamp: number;
  isOwn: boolean;
  sendState: SendState;
  /** A history-gap marker row (plan E2.5, D23): the peer's entries through
   *  this seq were retired before we fetched them; null for a message. */
  historyGapThrough: number | null;
}

/** An incoming friend request, waiting for our answer. */
export interface PendingRequest {
  publicKey: string;
  displayName: string;
  message: string;
  receivedAt: number;
}

/** A blocked identity. */
export interface BlockedUser {
  publicKey: string;
  displayName: string;
  blockedAt: number;
}

/** Our safety number with a peer and whether we verified their key
 *  (`dto::friends::SafetyNumberView`, plan E2.3). */
export interface SafetyNumberView {
  peerKey: string;
  /** The 60 digits both sides see, shown as 12 groups of 5. */
  digits: string;
  /** Signal's scannable form, base64. */
  scannable: string;
  /** The scannable form as a QR code: one string per row, `1` a dark
   *  module, without the quiet zone. */
  qrRows: string[];
  verified: boolean;
  verifiedAt: number | null;
  /** The key we verified changed and the change is not acknowledged. */
  changed: boolean;
}

/** Our friend code and its QR (`dto::friends::FriendCodeView`, plan E2.5
 *  D17): what `get_friend_code` and `rotate_friend_token` answer. */
export interface FriendCodeView {
  /** `rekindle://add/<public key>/<profile key>/<token>`. */
  url: string;
  publicKey: string;
  profileKey: string;
  token: string;
  /** The code as a QR code: one string per row, `1` a dark module,
   *  without the quiet zone. */
  qrRows: string[];
}

/** A friend invite we made that is still open. */
export interface OutgoingInvite {
  inviteId: string;
  url: string;
  createdAt: number;
  expiresAt: number;
  status: string;
  acceptedBy: string | null;
}

/** A new friend invite link. */
export interface FriendInviteCreated {
  url: string;
  inviteId: string;
  /** Milliseconds; the backend's one invite TTL decides `expiresAt`. */
  createdAt: number;
  expiresAt: number;
}

/** A freshly minted invite. */
export interface InviteCreated {
  inviteId: string;
  codeHash: string;
  url: string;
  /** Unix seconds. */
  expiresAt: number | null;
}

// ── Onboarding ──

export interface OnboardingConfig {
  enabled: boolean;
  mode: "default" | "guided" | "gated";
  defaultChannels: string[];
  questions: OnboardingQuestion[];
  welcomeMessage: string | null;
  guideSteps: GuideStep[];
}

export interface OnboardingQuestion {
  questionId: string;
  title: string;
  description: string | null;
  required: boolean;
  singleSelect: boolean;
  options: OnboardingOption[];
}

export interface OnboardingOption {
  optionId: string;
  title: string;
  description: string | null;
  rolesToAssign: number[];
  channelsToShow: string[];
}

export interface GuideStep {
  title: string;
  description: string;
  channelId: string | null;
  emoji: string | null;
}

export interface WelcomeScreen {
  description: string;
  channels: WelcomeChannelEntry[];
}

export interface WelcomeChannelEntry {
  channelId: string;
  description: string;
  emoji: string | null;
}

export interface OnboardingAnswer {
  questionId: string;
  selectedOptions: string[];
}

export interface GossipDiagnostics {
  communityId: string;
  hasGossip: boolean;
  gossipPeerCount: number;
  onlineMemberCount: number;
  knownMemberCount: number;
  needsInitialSync: boolean;
  messageClock: number;
  governanceClock: number;
  hasRouteBlob: boolean;
  myPseudonymKey: string | null;
  mySubkeyIndex: number | null;
  hasSlotKeypair: boolean;
  hasSlotSeed: boolean;
  hasMek: boolean;
  governanceKey: string | null;
  gossipPeerKeys: string[];
  onlineMemberKeys: string[];
}

/**
 * Backend lifecycle FSM state (`setup.rs` emits `{ state, at_ms }`).
 *
 * The store derives a view of this; it is not the store's own type.
 */
export type LifecycleState =
  | "stopped"
  | "starting"
  | "locked"
  | "resuming"
  | "operational"
  | "degraded"
  | "detached"
  | "locking"
  | "shutting_down";

// ── Host calls (plan E1.1) ──
//
// Every Host-call command answers `Done<T>` or rejects with a
// `CommandError` (`rekindle_types::dto::command`). `CommandError` is
// adjacently tagged (`{ type, data }`); the `IntentReview` inside it is
// externally tagged like every `rekindle_types::intent` enum
// (`{ variant: {...} }` or a bare string), pinned by the Rust golden files.

export type IntentKind = string;

type IntentClass =
  | "routine"
  | "consequential"
  | "trustChanging"
  | "destructive"
  | "protected";

type ProofRequired = "none" | "commit" | "acknowledge" | "typeSubject" | "passphrase";

export type Subject =
  | { person: { localNickname: string | null; displayName: string | null; fingerprint: string } }
  | { community: { id: string; name: string } }
  | { channel: { communityName: string; id: string; name: string } }
  | { role: { id: string; name: string } }
  | { file: { name: string } }
  | { link: { host: string } }
  | { ownIdentity: { name: string } }
  | { named: { name: string } };

export type Fact =
  | "subjectNotTold"
  | "pendingMessagesDropped"
  | "sessionDeleted"
  | "sessionKeptUntilReplaced"
  | "unopenedMessagesLost"
  | "presenceVisible"
  | "ipAddressLeaves"
  | "rejoinNeedsInvite"
  | { fingerprint: { fingerprint: string } }
  | { safetyNumber: { safetyNumber: string } }
  | { safetyNumberChanged: { oldSafetyNumber: string; newSafetyNumber: string } }
  | { membersAffected: { count: number } }
  | { messagesDeleted: { count: number } };

export type Escalation =
  | "firstMediaInCall"
  | "lastAdmin"
  | "soleMember"
  | "startsMekBallot"
  | { attachment: { fileName: string; metadataRemoved: boolean; sizeBytes: number } }
  | { containerNotEmpty: { members: number; messages: number } }
  | { crossAudience: { fromMembers: number; toMembers: number } }
  | { massMention: { notified: number } }
  | { pendingWork: { inCall: boolean; unsent: number; uploading: boolean } }
  | { verifiedKeyChanged: { oldSafetyNumber: string; newSafetyNumber: string } };

export type Reversibility =
  | "undoable"
  | "needsOthers"
  | "never"
  | { reversibleByInverse: IntentKind };

export type Notifies = "nobody" | "subject" | { members: number };

/** A held action, as every frontend renders it (ADR 0019). */
export interface IntentReview {
  intentId: string;
  kind: IntentKind;
  class: IntentClass;
  subject: Subject;
  facts: Fact[];
  escalations: Escalation[];
  reversibility: Reversibility;
  notifies: Notifies;
  proof: ProofRequired;
  expiresAtMs: number;
  /** Cancelling it runs an action of its own (the decline a session-reset
   *  request gets): only the explicit Cancel control cancels it, never Esc
   *  or a click outside the dialog. */
  cancelSends: boolean;
}

/** The proof a commit carries. */
export type IntentProof =
  | "commit"
  | "acknowledged"
  | { typed: string }
  | { passphrase: string };

/** How an action that ran can be taken back while it lasts. */
export interface UndoOffer {
  id: string;
  inverse: IntentKind;
  expiresAtMs: number;
}

/** A Host call that ran. */
export interface Done<T> {
  value: T;
  undo: UndoOffer | null;
}

/** A Host call that did not run. */
export type CommandError =
  | { type: "failed"; data: { code: number; message: string; remediation: string | null } }
  | { type: "intentRequired"; data: IntentReview }
  /** The user cancelled the review the call waited on; the Host answers it
   *  from `intent_cancel`. */
  | { type: "cancelled" };

/** Whether `e` is a `CommandError` (the shape a Host call rejects with). */
export function isCommandError(e: unknown): e is CommandError {
  if (typeof e !== "object" || e === null || !("type" in e)) return false;
  const type = (e as { type: unknown }).type;
  if (type === "cancelled") return true;
  return "data" in e && (type === "failed" || type === "intentRequired");
}

/** Whether `e` is the Host's answer to a cancelled review: the call did not
 *  run because the user cancelled it, which is not an error to show. */
export function isCancelled(e: unknown): boolean {
  return isCommandError(e) && e.type === "cancelled";
}

// ── Identity (rekindle_types::dto::identity) ──

/** One identity on this device, for the login picker. */
export interface IdentitySummary {
  publicKey: string;
  displayName: string;
  createdAt: number;
}

/** The identity a create or an unlock opened. */
export interface IdentityView {
  publicKey: string;
  displayName: string;
}

/** The unlocked identity's public data. */
export interface IdentityShown {
  publicKey: string;
  displayName: string;
  profileDhtKey: string;
  mailboxDhtKey: string;
  friendListDhtKey: string;
  communities: number;
}

// ── Our presence (rekindle_types::dto::presence) ──

export type SessionStatus = "online" | "away" | "busy" | "offline" | "invisible";

type IdleState = "unavailable" | "active" | "autoAway";

/** The game we play, detected or set by hand. */
export interface OwnGame {
  gameId: number;
  gameName: string;
  elapsedSeconds: number;
  serverAddress: string | null;
}

/** What `PresenceGet` answers. */
export interface OwnPresenceView {
  status: SessionStatus;
  statusMessage: string;
  game: OwnGame | null;
  idle: IdleState;
}

// ── The audit log (rekindle_types::dto::audit) ──

export type AuditVerified =
  | { state: "intact"; length: number }
  | { state: "broken"; length: number; at: number | null; detail: string };

export interface AuditEntryView {
  cursor: number;
  prevMac: string;
  mac: string;
  atMs: number;
  actorPub: string;
  kind: string;
  payload: unknown;
}
