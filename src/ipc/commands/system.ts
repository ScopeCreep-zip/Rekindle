import { hostInvoke, invoke } from "../invoke";
import type {
  ActivityListItem, FavouriteServerView, HiddenGameView, InstalledGameView, JoinSource, NetworkStatus, Preferences, QuickWipeState, SettingsTab,
} from "./types";
import type { OnboardingConfig, OnboardingConfigEdit, WelcomeScreen, OnboardingAnswer, GossipDiagnostics } from "./dto";
import type {
  AuditExported, AuditVerified, GameTotalView, LifecycleState, OwnGame, OwnPresenceView, SessionStatus,
  SkinApplied, SkinApplyFailure, SkinEntry, SkinLibrary,
} from "./dto";
import type { RegionFit, SkinDraft, SkinDraftReport } from "./dto_skin_draft";
import type { InfoviewRequest, InfoviewView } from "./dto_infoview";
import type { SkinSelection } from "./dto_skin_run";
import type { SkinAction } from "./dto_skin_tree";

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
  /// Our bio and a friend's (plan E7.2); `null` when there is none. Setting
  /// a blank one removes it; the Host sends it to every accepted friend.
  getBio: () => hostInvoke<string | null>("get_bio"),
  getFriendBio: (publicKey: string) => hostInvoke<string | null>("get_friend_bio", { publicKey }),
  setBio: (bio: string) => hostInvoke<string | null>("set_bio", { bio }),
  /// `kind` declares the picture's format; the Host decodes it as that
  /// kind only (`ImageKind`).
  setAvatar: (avatarData: number[], kind: ImageKind) =>
    hostInvoke<unknown>("set_avatar", { avatarData, kind }),
  getAvatar: (publicKey: string) => hostInvoke<number[] | null>("get_avatar", { publicKey }),
  setStatusMessage: (message: string) =>
    hostInvoke<OwnPresenceView>("set_status_message", { message }),

  // The audit log: Host calls.
  auditVerify: () => hostInvoke<AuditVerified>("audit_verify"),
  // The shell asks where to save; the Host writes the file (Protected).
  // `null` when the user cancelled the save dialog.
  auditExport: (since: number) => hostInvoke<AuditExported | null>("audit_export", { since }),

  // The skin this window shows (plan E7.7.2): a Host call; `null` asks
  // for the skin this device shows now.
  getSkin: (id: string | null = null) => hostInvoke<SkinApplied>("get_skin", { id }),
  // Settings → Appearance: Host calls. A library skin chosen goes on
  // trial; the Host reverts it unless `keepSkin` keeps it.
  listSkins: () => hostInvoke<SkinLibrary>("list_skins"),
  importSkin: (bytes: number[]) => hostInvoke<SkinEntry>("import_skin", { bytes }),
  removeSkin: (id: string) => hostInvoke<SkinLibrary>("remove_skin", { id }),
  selectSkin: (id: string) => hostInvoke<SkinLibrary>("select_skin", { id }),
  keepSkin: (id: string, keep: boolean) => hostInvoke<SkinLibrary>("keep_skin", { id, keep }),
  setLoginSkin: (id: string) => hostInvoke<SkinLibrary>("set_login_skin", { id }),
  // The skin editor (plan E7.7.5): every check is the Host's.
  validateSkinDraft: (draft: SkinDraft) =>
    hostInvoke<SkinDraftReport>("validate_skin_draft", { draft }),
  skinDraftStart: (id: string | null = null) => hostInvoke<SkinDraft>("skin_draft_start", { id }),
  // The shell asks where to save; `null` when the user cancelled.
  exportSkinDraft: (draft: SkinDraft) => hostInvoke<string | null>("export_skin_draft", { draft }),
  // Region import: the shell asks for the image, the Host validates the
  // draft with it; the draft with the region, or `null` if cancelled.
  importSkinRegion: (draft: SkinDraft, region: string, fit: RegionFit, textBacking: string | null) =>
    hostInvoke<SkinDraft | null>("import_skin_region", { draft, region, fit, textBacking }),
  // The token file's JSON Schema for DTCG tools; `false` if cancelled.
  exportSkinSchema: () => hostInvoke<boolean>("export_skin_schema"),
  // A skin node's trigger (plan SK2-A4): a feature answers its own
  // request's value; any other a SkinRun for the window's shell.
  // The window is the calling one, by label (lead decision 48).
  runSkinAction: (node: string, action: SkinAction) =>
    hostInvoke<unknown>("run_skin_action", { node, action }),
  setSkinSelection: (selection: SkinSelection | null) =>
    hostInvoke<void>("set_skin_selection", { selection }),
  // This window could not apply `skin` and kept the one it shows; the
  // Host logs it and reverts a skin on trial (plan SK2-T1).
  reportSkinApplyFailed: (skin: string, cause: SkinApplyFailure) =>
    hostInvoke<void>("report_skin_apply_failed", { skin, cause }),

  // Game
  getGameStatus: () => hostInvoke<OwnGame | null>("get_game_status"),
  // Our time in each detected game, the most recent first (plan E7.2).
  getGameTotals: () => hostInvoke<GameTotalView[]>("get_game_totals"),
  // What a friend last shared (empty when they share none).
  getFriendGameTotals: (publicKey: string) =>
    hostInvoke<GameTotalView[]>("get_friend_game_totals", { publicKey }),
  getHiddenGames: () => hostInvoke<HiddenGameView[]>("get_hidden_games"),
  hideGame: (gameId: number) => hostInvoke<HiddenGameView[]>("hide_game", { gameId }),
  unhideGame: (gameId: number) => hostInvoke<HiddenGameView[]>("unhide_game", { gameId }),
  getGameName: (gameId: number) => invoke<string | null>("get_game_name", { gameId }),
  /** The one launch path (`GameLaunch`): `server` null only for "installed". */
  launchGame: (gameId: number, server: string | null, source: JoinSource) =>
    hostInvoke<null>("launch_game", { gameId, server, source }),
  getInstalledGames: () => hostInvoke<InstalledGameView[]>("get_installed_games"),
  sendGameInvite: (publicKey: string, gameId: number, server: string) =>
    hostInvoke<string>("send_game_invite", { publicKey, gameId, server }),
  getFavouriteServers: () => hostInvoke<FavouriteServerView[]>("get_favourite_servers"),
  /// The calling window's Infoview of `subject`; Rust names the window.
  getInfoview: (subject: InfoviewRequest) => hostInvoke<InfoviewView>("get_infoview", { subject }),
  addFavouriteServer: (gameId: number, server: string, label: string) =>
    hostInvoke<FavouriteServerView[]>("add_favourite_server", { gameId, server, label }),
  removeFavouriteServer: (gameId: number, server: string) =>
    hostInvoke<FavouriteServerView[]>("remove_favourite_server", { gameId, server }),

  // Settings
  getPreferences: () => invoke<Preferences>("get_preferences"),
  // A trust-changing intent the Host reviews; the preferences come back.
  setReadReceiptsEnabled: (enabled: boolean) =>
    hostInvoke<Preferences>("set_read_receipts_enabled", { enabled }),
  setShareGameTotals: (enabled: boolean) =>
    hostInvoke<Preferences>("set_share_game_totals", { enabled }),
  setGameDetection: (enabled: boolean) =>
    hostInvoke<Preferences>("set_game_detection", { enabled }),
  // The local activity feed (plan E7.15): one page, newest first, after
  // row `before`; keeping it or not (off clears it). Host calls.
  activityList: (before: number | null, limit: number) =>
    hostInvoke<ActivityListItem[]>("activity_list", { before, limit }),
  setKeepActivityLog: (enabled: boolean) =>
    hostInvoke<Preferences>("set_keep_activity_log", { enabled }),
  setShareCurrentGame: (enabled: boolean) =>
    hostInvoke<Preferences>("set_share_current_game", { enabled }),
  // The quick wipe (plan E7.1): Protected intents the Host reviews.
  quickWipeArm: (duressPassphrase: string, shortcut: string | null) =>
    hostInvoke<QuickWipeState>("quick_wipe_arm", { duressPassphrase, shortcut }),
  quickWipeDisarm: () => hostInvoke<QuickWipeState>("quick_wipe_disarm"),
  setPreferences: (prefs: Preferences) =>
    invoke<void>("set_preferences", { prefs }),
  checkForUpdates: () => invoke<boolean>("check_for_updates"),

  // Notifications — backend command instead of the plugin's `sendNotification`,
  // whose Linux `show` path panics (blocks on zbus inside the async runtime).

  // Windows
  showBuddyList: () => invoke<void>("show_buddy_list"),
  /** Quit Rekindle, the tray's Quit (closing the buddy list only hides it). */
  quitApp: () => invoke<void>("quit_app"),
  // Window titles are resolved by the backend from its own state. One chat
  // window: 1:1 with a peer, or a group DM by its group id (plan E2.6).
  openChatWindow: (publicKey: string) =>
    invoke<void>("open_chat_window", { target: { peer: publicKey } }),
  openGroupChatWindow: (groupId: string) =>
    invoke<void>("open_chat_window", { target: { group: groupId } }),
  openSettingsWindow: (tab?: SettingsTab) =>
    invoke<void>("open_settings_window", { tab: tab ?? null }),
  /// The skin editor, a Settings sub-window (plan E7.7.5).
  openSkinEditorWindow: () => invoke<void>("open_skin_editor_window"),
  /// Open, or close if open, the pack popup `popup` beside this skin window (plan SK2-A5).
  openSkinPopup: (popup: string) => invoke<void>("open_skin_popup", { popup }),
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
  setOnboardingConfig: (communityId: string, config: OnboardingConfigEdit) =>
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
