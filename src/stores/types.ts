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
  coordinatorPseudonym: string | null;
  gossipPeerKeys: string[];
  onlineMemberKeys: string[];
}
