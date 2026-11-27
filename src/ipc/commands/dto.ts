import type { SkinShell } from "./dto_skin_tree";
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

import type { NotJoinableReason, SendState, SessionStatus } from "./types";

export interface GameInfo {
  gameName: string;
  gameId: number | null;
  startedAt: number | null;
  serverAddress: string | null;
  iconKey: string | null; // `src/assets/game-icons/<key>.png`, from the backend (plan E7.2)
  /** The Host's join check (plan E7.15 IV3c), from the friend list's
   *  `GameStatus` and from each live presence event. */
  joinable: boolean;
  joinableReason: NotJoinableReason | null;
}

/** One audit-log row (`rekindle_types::dto::community::CommunityAuditEntry`). */
export interface CommunityAuditEntry {
  action: string;
  actorPseudonym: string;
  /** The actor's name as the Host states it, never empty (card name, else short fingerprint; E3.4 S6d). */
  actorName: string;
  target: string | null;
  /** The target's name as the Host states it; absent when the target is not a counted member. */
  targetName?: string;
  details: string | null;
  timestamp: number;
}

/** A banned member (`rekindle_types::dto::community::CommunityBanView`). */
export interface CommunityBanView {
  pseudonymKey: string;
  /** The roster's name, or a short form of the key. */
  displayName: string;
}

/** A live community invite (`rekindle_types::dto::community::CommunityInviteView`). */
export interface InviteDto {
  /** What a revoke names. */
  inviteId: string;
  codeHash: string;
  createdBy: string;
  maxUses: number;
  /** Counted only by the minter. */
  uses: number;
  /** Unix seconds. */
  expiresAt: number | null;
  /** Milliseconds; null for another member's invite. */
  createdAt: number | null;
  /** The invite link, only for invites this identity minted. */
  url: string | null;
  /** The fingerprint the joiner's review shows for this invite (the Host's
   *  `invite_fingerprint`), compared out of band like a safety number. */
  fingerprint: string;
}

// ── Friends (plan E2.1; `rekindle_types::dto::friends`; direct messages are `dto_dm.ts`) ──

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
  rolesToAssign: string[];
  channelsToShow: string[];
}

/**
 * The flow the editor sends (`set_onboarding_config`, plan E7.15 S3): a new
 * question or option carries no id (`null`); the Host mints it and keeps an
 * existing item's id. A loaded `OnboardingConfig` is a valid edit.
 */
export interface OnboardingConfigEdit extends Omit<OnboardingConfig, "questions"> {
  questions: OnboardingQuestionEdit[];
}

export interface OnboardingQuestionEdit extends Omit<OnboardingQuestion, "questionId" | "options"> {
  questionId: string | null;
  options: OnboardingOptionEdit[];
}

export interface OnboardingOptionEdit extends Omit<OnboardingOption, "optionId"> {
  optionId: string | null;
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
  messageClock: number;
  hasRouteBlob: boolean;
  myPseudonymKey: string | null;
  mySubkeyIndex: number | null;
  hasSlotKeypair: boolean;
  hasSlotSeed: boolean;
  hasMek: boolean;
  anchor: string | null;
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
  | "rejoinNeedsInvite"
  | "noHistoryCopied"
  | { ipAddressLeaves: { to: Recipient } }
  | { newCreatorSet: { creators: string[] } }
  | { creatorsLeave: { creators: string[] } }
  | { successorLinked: { signer: string } }
  | { transferLinksMissing: { creators: string[] } }
  | { fingerprint: { fingerprint: string } }
  | { safetyNumber: { safetyNumber: string } }
  | { safetyNumberChanged: { oldSafetyNumber: string; newSafetyNumber: string } }
  | { keyChanged: { oldFingerprint: string; newFingerprint: string } }
  | { membersAffected: { count: number } }
  | { messagesDeleted: { count: number } };

export type Escalation =
  | "firstMediaInCall"
  | "lastAdmin"
  | "soleMember"
  | "startsMekBallot"
  | "visibleBeforeUnlock"
  | { attachment: { fileName: string; metadataRemoved: boolean; sizeBytes: number } }
  | { containerNotEmpty: { members: number; messages: number } }
  | { crossAudience: { fromMembers: number; toMembers: number } }
  | { massMention: { notified: number } }
  | { pendingWork: { inCall: boolean; unsent: number; uploading: boolean } }
  | { verifiedKeyChanged: { oldSafetyNumber: string; newSafetyNumber: string } };

/** The third party our IP address reaches (`Recipient`). */
export type Recipient =
  | { site: { host: string } }
  | { gameServer: { game: string; address: string } };

type RecipientKind = "site" | "gameServer";

/** Why an action can never be taken back (`IrreversibleReason`). */
export type IrreversibleReason = { disclosureCannotBeRecalled: { to: RecipientKind } };

export type Reversibility =
  | "undoable"
  | "needsOthers"
  | "never"
  | { neverBecause: IrreversibleReason }
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
  /** The Host's own statement for an Acknowledge proof's checkbox, never
   *  a fact line; null for any other proof. */
  acknowledgement: string | null;
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
  /** The identity's fingerprint form, as the Host formats it for the
   *  member list (contract VN2): how peers see this identity. */
  fingerprint: string;
  createdAt: number;
}

/** The identity a create or an unlock opened. */
export interface IdentityView {
  publicKey: string;
  displayName: string;
  /** The identity's fingerprint form, as the Host formats it for the
   *  member list (contract VN2): how peers see this identity. */
  fingerprint: string;
}

/** The unlocked identity's public data. */
export interface IdentityShown {
  publicKey: string;
  displayName: string;
  /** The identity's fingerprint form, as the Host formats it for the
   *  member list (contract VN2): how peers see this identity. */
  fingerprint: string;
  /** Null while not yet held (the own-records stage names each after unlock). */
  profileDhtKey: string | null;
  mailboxDhtKey: string | null;
  friendListDhtKey: string | null;
  communities: number;
}

// ── Our presence (rekindle_types::dto::presence) ──

export type { SessionStatus } from "./types";

type IdleState = "unavailable" | "active" | "autoAway";

/** The game we play, detected or set by hand. */
export interface OwnGame {
  gameId: number;
  gameName: string;
  elapsedSeconds: number;
  serverAddress: string | null;
  iconKey: string | null;
}

/** Our time in one game (`GameTotalsGet`); `lastPlayed` is Unix ms, `iconKey` as on `GameInfo`. */
export interface GameTotalView {
  gameId: number;
  gameName: string;
  iconKey: string | null;
  totalSecs: number;
  lastPlayed: number;
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

/// What the Host wrote for `auditExport`: the file and its entry count.
export interface AuditExported {
  path: string;
  entries: number;
}

/// One skin the user can choose (`rekindle_types::dto::skin_library::SkinEntry`).
export interface SkinEntry {
  /// `builtin:<id>` or `pkg:<hash>`.
  id: string;
  name: string;
  /// When it entered the library (ms); `null` for a built-in.
  importedAt: number | null;
  /// Who made it, as the skin states it; a library skin's are self-claimed.
  credits: SkinCredits | null;
  /// Controls served under 24 CSS px with no spacing exception (WCAG 2.2 SC 2.5.8).
  shortTargets: number;
  /// `shortTargets === 0`: every control meets SC 2.5.8.
  conforming: boolean;
}

/// A skin's human credits (`rekindle_types::dto::skin::SkinCredits`).
export interface SkinCredits {
  /// The original authors.
  authors: string[];
  contributors: string[];
  /// Where the skin was published: shown as text, opened only through
  /// `open_external_url`.
  url: string | null;
  /// The licence as the skin states it, or `"noneStated"`.
  licence: { text: string } | "noneStated";
}

/// A library skin on trial ("Keep this skin?", `SkinTrial`): the Host
/// reverts it to `previous` at `revertsAtMs` unless it is kept.
export interface SkinTrial {
  id: string;
  name: string;
  previous: string;
  revertsAtMs: number;
}

/// Why a window could not apply a skin (`SkinApplyFailure`): the window
/// reports it with `report_skin_apply_failed`, and the Host decides.
export type SkinApplyFailure =
  | { type: "imageDecode"; data: { path: string } }
  | { type: "sheetError"; data: { detail: string } };

/// `list_skins`' answer (`SkinLibrary`).
export interface SkinLibrary {
  skins: SkinEntry[];
  /// The identity's stored skin.
  active: string;
  /// The device login skin: `builtin:<id>` or `dev:<hash>`.
  login: string;
  trial: SkinTrial | null;
}

/// One TUI colour role and its opaque sRGB value
/// (`rekindle_types::dto::skin_library::SkinPaletteColour`).
interface SkinPaletteColour {
  role: string;
  rgb: [number, number, number];
}

/// What a window applies (`rekindle_types::dto::skin_library::SkinApplied`,
/// plan E7.7.2): `get_skin`'s answer and `SkinChanged`'s payload.
export interface SkinApplied {
  /// `builtin:<id>`, `pkg:<hash>` or `dev:<hash>`.
  id: string;
  /// The active variant's base, for `:root[data-skin-base]`.
  base: "light" | "dark";
  /// The `skin://` href of the `@layer skin` stylesheet.
  tokensHref: string;
  /// The region slots the skin paints.
  regions: string[];
  /// The `skin://` href of every image the stylesheet draws: a window
  /// decodes them all before it shows the sheet.
  images: string[];
  /// The TUI's palette subset (the webview reads the stylesheet instead).
  palette: SkinPaletteColour[];
  /// Each tree-laid window kind's outline (plan SK2-S2); absent when the
  /// skin lays none (the built-in default keeps the app's own layout).
  shells?: SkinShell[];
  /// The brand glyph the shell draws inline in the title strip's logo box
  /// (the first strip logo's, Main before Chat), chosen by the Host from that
  /// box (plan BR); absent with no strip logo.
  titleLogo?: BrandGlyphName;
}

/** The brand glyphs a title-strip logo box takes (the `NodeGlyph`s
 *  `skin_tree::brand::glyph` returns). */
export type BrandGlyphName = "rekindle" | "rekindleMarkSmall" | "rekindleWordmark";

/** The member-list group the Host put a member in (Tier 1 `MemberGroup`):
 *  every frontend renders it as given. */
export type MemberGroup =
  | { kind: "role"; id: string; name: string }
  | { kind: "online" }
  | { kind: "offline" };

/** One field of a community profile edit (plan PF2, `rekindle_types::dto::community_profile::ProfileField`). */
export type ProfileField<T> = { op: "unchanged" } | { op: "clear" } | { op: "set"; value: T };

/** `update_community_profile`'s patch: every field explicit (PF2). */
export interface CommunityProfilePatch {
  bio: ProfileField<string>;
  pronouns: ProfileField<string>;
  themeColor: ProfileField<number>;
  badges: ProfileField<string[]>;
  avatarRef: ProfileField<string>;
  bannerRef: ProfileField<string>;
  customStatus: ProfileField<string>;
  displayName: ProfileField<string>;
}
