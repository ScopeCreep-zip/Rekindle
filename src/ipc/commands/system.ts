import { invoke } from "../invoke";
import type {
  GameStatus, NetworkStatus, Preferences,
} from "./types";
import type { OnboardingConfig, WelcomeScreen, OnboardingAnswer, GossipDiagnostics } from "../../stores/types";
import type { LifecycleState } from "../../stores/lifecycle.store";

export const systemCommands = {
  // Status
  setStatus: (status: string) => invoke<void>("set_status", { status }),
  setNickname: (nickname: string) =>
    invoke<void>("set_nickname", { nickname }),
  setAvatar: (avatarData: number[]) =>
    invoke<void>("set_avatar", { avatarData }),
  getAvatar: (publicKey: string) =>
    invoke<number[] | null>("get_avatar", { publicKey }),
  setStatusMessage: (message: string) =>
    invoke<void>("set_status_message", { message }),

  // Game
  getGameStatus: () => invoke<GameStatus | null>("get_game_status"),
  getGameName: (gameId: number) => invoke<string | null>("get_game_name", { gameId }),
  launchGameToServer: (gameId: number, serverAddress: string) =>
    invoke<void>("launch_game_to_server", { gameId, serverAddress }),

  // Settings
  getPreferences: () => invoke<Preferences>("get_preferences"),
  setPreferences: (prefs: Preferences) =>
    invoke<void>("set_preferences", { prefs }),
  checkForUpdates: () => invoke<boolean>("check_for_updates"),

  // Windows
  showBuddyList: () => invoke<void>("show_buddy_list"),
  openChatWindow: (publicKey: string, displayName: string) =>
    invoke<void>("open_chat_window", { publicKey, displayName }),
  openSettingsWindow: (tab?: string) => invoke<void>("open_settings_window", { tab: tab ?? null }),
  openCommunityWindow: (communityId: string, communityName: string) =>
    invoke<void>("open_community_window", { communityId, communityName }),
  openProfileWindow: (publicKey: string, displayName: string) =>
    invoke<void>("open_profile_window", { publicKey, displayName }),
  /// Wave 12 W12.7 — pop the active call into its own webview window.
  openCallWindow: (callId: string) =>
    invoke<void>("open_call_window", { callId }),
  getNetworkStatus: () => invoke<NetworkStatus>("get_network_status"),
  // Lifecycle — current FSM state, used to seed the derived lifecycle store.
  lifecycleCurrent: () => invoke<LifecycleState>("lifecycle_current"),

  // Onboarding & Welcome Screen
  getOnboardingConfig: (communityId: string) =>
    invoke<OnboardingConfig>("get_onboarding_config", { communityId }),
  setOnboardingConfig: (communityId: string, config: OnboardingConfig) =>
    invoke<void>("set_onboarding_config", { communityId, config }),
  getWelcomeScreen: (communityId: string) =>
    invoke<WelcomeScreen>("get_welcome_screen", { communityId }),
  setWelcomeScreen: (communityId: string, screen: WelcomeScreen) =>
    invoke<void>("set_welcome_screen", { communityId, screen }),
  /**
   * Architecture §19.2 step 3 — `acknowledgedRules` must be `true`
   * when the merged `OnboardingConfig.mode === "gated"`, otherwise the
   * backend rejects the submission with a clear error. For
   * `default` / `guided` modes the flag is ignored.
   */
  submitOnboardingAnswers: (
    communityId: string,
    answers: OnboardingAnswer[],
    acknowledgedRules?: boolean,
  ) =>
    invoke<void>("submit_onboarding_answers", {
      communityId,
      answers,
      acknowledgedRules: acknowledgedRules ?? null,
    }),
  /** Plan §Failure 8 — persist `community_members.onboarding_complete = 1` for
   *  the local user so the wizard does not re-show on next launch. */
  markOnboardingComplete: (communityId: string) =>
    invoke<void>("mark_onboarding_complete", { communityId }),
  debugGossipState: (communityId: string) =>
    invoke<GossipDiagnostics>("debug_gossip_state", { communityId }),
};
