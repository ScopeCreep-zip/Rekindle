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
  code: string;
  createdBy: string;
  maxUses: number | null;
  uses: number;
  expiresAt: number | null;
  createdAt: number;
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
