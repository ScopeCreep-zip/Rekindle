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

export interface InviteDto {
  codeHash: string;
  createdBy: string;
  maxUses: number | null;
  uses: number;
  expiresAt: number | null;
  createdAt: number;
  /** Raw invite code — only available for invites this node created. */
  code?: string;
  /** VLD0 pointer to the encrypted InviteSecrets DFLT record. Carried in the
   *  deep link so the joiner fetches secrets directly. Local-only invites. */
  secretsRecordKey?: string;
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
  lamportCounter: number;
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
