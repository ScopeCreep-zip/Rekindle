import { hostInvoke, invoke } from "../invoke";
import type {
  NetworkStatus, Preferences, SettingsTab,
} from "./types";
import type { OnboardingConfig, WelcomeScreen, OnboardingAnswer, GossipDiagnostics } from "./dto";
import type {
  AuditEntryView, AuditVerified, LifecycleState, OwnGame, OwnPresenceView, SessionStatus,
} from "./dto";

/// The picture formats the Host accepts (`rekindle_types::ImageKind`,
/// serialized as its MIME type).
export type ImageKind = "image/png" | "image/jpeg" | "image/gif";

const IMAGE_KINDS: readonly ImageKind[] = ["image/png", "image/jpeg", "image/gif"];

/// `mime` as an [`ImageKind`], or `null` when the Host would refuse it.
export function asImageKind(mime: string): ImageKind | null {
  return IMAGE_KINDS.find((k) => k === mime) ?? null;
}

export const systemCommands = {
  // Our presence and profile: Host calls (plan E1.1). "Appear Offline" is
  // `invisible`; offline is set only by logging out.
  setStatus: (status: SessionStatus) => hostInvoke<OwnPresenceView>("set_status", { status }),
  setNickname: (nickname: string) => hostInvoke<string>("set_nickname", { nickname }),
  /// `kind` declares the picture's format; the Host decodes it as that
  /// kind only (`ImageKind`).
  setAvatar: (avatarData: number[], kind: ImageKind) =>
    hostInvoke<unknown>("set_avatar", { avatarData, kind }),
  getAvatar: (publicKey: string) => hostInvoke<number[] | null>("get_avatar", { publicKey }),
  setStatusMessage: (message: string) =>
    hostInvoke<OwnPresenceView>("set_status_message", { message }),

  // The audit log: Host calls.
  auditVerify: () => hostInvoke<AuditVerified>("audit_verify"),
  auditExport: (since: number) => hostInvoke<AuditEntryView[]>("audit_export", { since }),

  // Game
  getGameStatus: () => hostInvoke<OwnGame | null>("get_game_status"),
  getGameName: (gameId: number) => invoke<string | null>("get_game_name", { gameId }),
  launchGameToServer: (gameId: number, serverAddress: string) =>
    invoke<void>("launch_game_to_server", { gameId, serverAddress }),

  // Settings
  getPreferences: () => invoke<Preferences>("get_preferences"),
  // A trust-changing intent the Host reviews; the preferences come back.
  setReadReceiptsEnabled: (enabled: boolean) =>
    hostInvoke<Preferences>("set_read_receipts_enabled", { enabled }),
  setPreferences: (prefs: Preferences) =>
    invoke<void>("set_preferences", { prefs }),
  checkForUpdates: () => invoke<boolean>("check_for_updates"),

  // Notifications — backend command instead of the plugin's `sendNotification`,
  // whose Linux `show` path panics (blocks on zbus inside the async runtime).

  // Windows
  showBuddyList: () => invoke<void>("show_buddy_list"),
  // Window titles are resolved by the backend from its own state. One chat
  // window: 1:1 with a peer, or a group DM by its group id (plan E2.6).
  openChatWindow: (publicKey: string) =>
    invoke<void>("open_chat_window", { target: { peer: publicKey } }),
  openGroupChatWindow: (groupId: string) =>
    invoke<void>("open_chat_window", { target: { group: groupId } }),
  openSettingsWindow: (tab?: SettingsTab) =>
    invoke<void>("open_settings_window", { tab: tab ?? null }),
  /// `communityId` absent opens the community browser.
  openCommunityWindow: (communityId?: string) =>
    invoke<void>("open_community_window", { communityId: communityId ?? null }),
  openProfileWindow: (publicKey: string) => invoke<void>("open_profile_window", { publicKey }),
  /// Wave 12 W12.7 — pop the active call into its own webview window.
  openCallWindow: (callId: string) =>
    invoke<void>("open_call_window", { callId }),
  getNetworkStatus: () => invoke<NetworkStatus>("get_network_status"),
  /// Open an https link: the Host's review says the site will see our IP
  /// address; its commit opens the link. Resolves with the normalized URL.
  openExternalUrl: (url: string) => hostInvoke<string>("open_external_url", { url }),
  // Lifecycle — current FSM state, used to seed the derived lifecycle store.
  lifecycleCurrent: () => hostInvoke<LifecycleState>("lifecycle_current"),

  // Onboarding & Welcome Screen
  getOnboardingConfig: (communityId: string) =>
    hostInvoke<OnboardingConfig>("get_onboarding_config", { communityId }),
  setOnboardingConfig: (communityId: string, config: OnboardingConfig) =>
    hostInvoke<void>("set_onboarding_config", { communityId, config }),
  getWelcomeScreen: (communityId: string) =>
    hostInvoke<WelcomeScreen>("get_welcome_screen", { communityId }),
  setWelcomeScreen: (communityId: string, screen: WelcomeScreen) =>
    hostInvoke<void>("set_welcome_screen", { communityId, screen }),
  /**
   * Architecture §19.2 step 3 — `acknowledgedRules` must be `true`
   * when the merged `OnboardingConfig.mode === "gated"`, otherwise the
   * Host refuses the submission (400). For
   * `default` / `guided` modes the flag is ignored.
   */
  submitOnboardingAnswers: (
    communityId: string,
    answers: OnboardingAnswer[],
    acknowledgedRules?: boolean,
  ) =>
    hostInvoke<void>("submit_onboarding_answers", {
      communityId,
      answers,
      acknowledgedRules: acknowledgedRules ?? false,
    }),
  /** Plan §Failure 8 — persist `community_members.onboarding_complete = 1` for
   *  the local user so the wizard does not re-show on next launch. */
  markOnboardingComplete: (communityId: string) =>
    hostInvoke<void>("mark_onboarding_complete", { communityId }),
  debugGossipState: (communityId: string) =>
    invoke<GossipDiagnostics>("debug_gossip_state", { communityId }),
};
