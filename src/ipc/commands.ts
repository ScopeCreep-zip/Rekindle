import { invoke } from "./invoke";
import type { OnboardingConfig, WelcomeScreen, OnboardingAnswer } from "../stores/types";

export interface LoginResult {
  publicKey: string;
  displayName: string;
}

export interface IdentitySummary {
  publicKey: string;
  displayName: string;
  createdAt: number;
  hasAvatar: boolean;
  avatarBase64: string | null;
}

export interface Message {
  id: number;
  senderId: string;
  body: string;
  timestamp: number;
  isOwn: boolean;
  serverMessageId?: string;
  reactions?: { emoji: string; count: number; reactors: string[] }[];
  pinned?: boolean;
}

export interface FriendInfo {
  publicKey: string;
  displayName: string;
  nickname: string | null;
  status: string;
  statusMessage: string | null;
  gameInfo: GameStatus | null;
  group: string | null;
  unreadCount: number;
  lastSeenAt: number | null;
  friendshipState: "pendingOut" | "accepted";
}

export interface GameStatus {
  gameId: number;
  gameName: string;
  serverInfo: string | null;
  elapsedSeconds: number;
  serverAddress: string | null;
}

export interface AudioDeviceInfo {
  id: string;
  name: string;
  isDefault: boolean;
}

export interface AudioDevices {
  inputDevices: AudioDeviceInfo[];
  outputDevices: AudioDeviceInfo[];
}

export interface Preferences {
  notificationsEnabled: boolean;
  notificationSound: boolean;
  startMinimized: boolean;
  autoStart: boolean;
  gameDetectionEnabled: boolean;
  gameScanIntervalSecs: number;
  inputDevice: string | null;
  outputDevice: string | null;
  inputVolume: number;
  outputVolume: number;
  noiseSuppression: boolean;
  echoCancellation: boolean;
  autoAwayMinutes: number;
}

export interface NetworkStatus {
  attachmentState: string;
  isAttached: boolean;
  publicInternetReady: boolean;
  hasRoute: boolean;
  profileDhtKey: string | null;
  friendListDhtKey: string | null;
}

export const commands = {
  // Auth
  createIdentity: (passphrase: string, displayName?: string) =>
    invoke<LoginResult>("create_identity", { passphrase, displayName: displayName ?? null }),
  login: (publicKey: string, passphrase: string) =>
    invoke<LoginResult>("login", { publicKey, passphrase }),
  getIdentity: () => invoke<LoginResult | null>("get_identity"),
  logout: () => invoke<void>("logout"),
  listIdentities: () => invoke<IdentitySummary[]>("list_identities"),
  deleteIdentity: (publicKey: string, passphrase: string) =>
    invoke<void>("delete_identity", { publicKey, passphrase }),

  // Chat
  prepareChatSession: (peerId: string) =>
    invoke<void>("prepare_chat_session", { peerId }),
  sendMessage: (to: string, body: string) =>
    invoke<void>("send_message", { to, body }),
  sendTyping: (peerId: string, typing: boolean) =>
    invoke<void>("send_typing", { peerId, typing }),
  getMessageHistory: (peerId: string, limit: number) =>
    invoke<Message[]>("get_message_history", { peerId, limit }),
  markRead: (peerId: string) => invoke<void>("mark_read", { peerId }),

  // Friends
  addFriend: (publicKey: string, displayName: string, message: string) =>
    invoke<void>("add_friend", { publicKey, displayName, message }),
  removeFriend: (publicKey: string) =>
    invoke<void>("remove_friend", { publicKey }),
  acceptRequest: (publicKey: string, displayName: string) =>
    invoke<void>("accept_request", { publicKey, displayName }),
  rejectRequest: (publicKey: string) =>
    invoke<void>("reject_request", { publicKey }),
  getFriends: () => invoke<FriendInfo[]>("get_friends"),
  getPendingRequests: () =>
    invoke<{ publicKey: string; displayName: string; message: string; receivedAt: number }[]>(
      "get_pending_requests",
    ),
  createFriendGroup: (name: string) =>
    invoke<number>("create_friend_group", { name }),
  renameFriendGroup: (groupId: number, name: string) =>
    invoke<void>("rename_friend_group", { groupId, name }),
  moveFriendToGroup: (publicKey: string, groupId: number | null) =>
    invoke<void>("move_friend_to_group", { publicKey, groupId }),
  generateInvite: () =>
    invoke<{ url: string; inviteId: string }>("generate_invite"),
  addFriendFromInvite: (inviteString: string) =>
    invoke<void>("add_friend_from_invite", { inviteString }),
  cancelInvite: (inviteId: string) =>
    invoke<void>("cancel_invite", { inviteId }),
  getOutgoingInvites: () =>
    invoke<{ inviteId: string; url: string; createdAt: number; expiresAt: number; status: string; acceptedBy: string | null }[]>(
      "get_outgoing_invites",
    ),
  blockUser: (publicKey: string, displayName?: string) =>
    invoke<void>("block_user", { publicKey, displayName: displayName ?? null }),
  unblockUser: (publicKey: string) =>
    invoke<void>("unblock_user", { publicKey }),
  getBlockedUsers: () =>
    invoke<{ publicKey: string; displayName: string; blockedAt: number }[]>("get_blocked_users"),
  cancelRequest: (publicKey: string) =>
    invoke<void>("cancel_request", { publicKey }),
  emitFriendsPresence: () =>
    invoke<void>("emit_friends_presence"),

  // Community
  getCommunities: () =>
    invoke<{ id: string; name: string; channelCount: number }[]>("get_communities"),
  getCommunityDetails: () =>
    invoke<{
      id: string;
      name: string;
      description: string | null;
      channels: { id: string; name: string; channelType: string; unreadCount: number; categoryId?: string; topic?: string }[];
      categories?: { id: string; name: string; sortOrder: number }[];
      myRole: string | null;
      myRoleIds: number[];
      roles: { id: number; name: string; color: number; permissions: string; position: number; hoist: boolean; mentionable: boolean }[];
      myPseudonymKey: string | null;
      mekGeneration: number;
    }[]>("get_community_details"),
  getCommunityMembers: (communityId: string) =>
    invoke<{ pseudonymKey: string; displayName: string; roleIds: number[]; displayRole: string; status: string; timeoutUntil: number | null }[]>(
      "get_community_members", { communityId },
    ),
  createCommunity: (name: string) =>
    invoke<string>("create_community", { name }),
  joinCommunity: (communityId: string, inviteCode?: string) =>
    invoke<void>("join_community", { communityId, inviteCode: inviteCode ?? null }),
  createChannel: (communityId: string, name: string, channelType: string, categoryId?: string) =>
    invoke<string>("create_channel", { communityId, name, channelType, categoryId: categoryId ?? null }),
  sendChannelMessage: (channelId: string, body: string, replyToId?: string) =>
    invoke<string>("send_channel_message", { channelId, body, replyToId: replyToId ?? null }),
  editChannelMessage: (channelId: string, messageId: string, newBody: string) =>
    invoke<void>("edit_channel_message", { channelId, messageId, newBody }),
  deleteChannelMessage: (channelId: string, messageId: string) =>
    invoke<void>("delete_channel_message", { channelId, messageId }),
  getChannelMessages: (channelId: string, limit: number) =>
    invoke<Message[]>("get_channel_messages", { channelId, limit }),
  getOlderChannelMessages: (communityId: string, channelId: string, beforeTimestamp: number, limit: number) =>
    invoke<Message[]>("get_older_channel_messages", { communityId, channelId, beforeTimestamp, limit }),
  removeCommunityMember: (communityId: string, pseudonymKey: string) =>
    invoke<void>("remove_community_member", { communityId, pseudonymKey }),
  leaveCommunity: (communityId: string) =>
    invoke<void>("leave_community", { communityId }),
  deleteChannel: (communityId: string, channelId: string) =>
    invoke<void>("delete_channel", { communityId, channelId }),
  renameChannel: (communityId: string, channelId: string, newName: string) =>
    invoke<void>("rename_channel", { communityId, channelId, newName }),
  updateCommunityInfo: (communityId: string, name: string | null, description: string | null) =>
    invoke<void>("update_community_info", { communityId, name, description }),
  banMember: (communityId: string, pseudonymKey: string) =>
    invoke<void>("ban_member", { communityId, pseudonymKey }),
  unbanMember: (communityId: string, pseudonymKey: string) =>
    invoke<void>("unban_member", { communityId, pseudonymKey }),
  getBanList: (communityId: string) =>
    invoke<{ pseudonymKey: string; displayName: string; bannedAt: number }[]>(
      "get_ban_list", { communityId },
    ),
  rotateMek: (communityId: string) =>
    invoke<void>("rotate_mek", { communityId }),

  // Reactions
  addReaction: (communityId: string, channelId: string, messageId: string, emoji: string) =>
    invoke<void>("add_reaction", { communityId, channelId, messageId, emoji }),
  removeReaction: (communityId: string, channelId: string, messageId: string, emoji: string) =>
    invoke<void>("remove_reaction", { communityId, channelId, messageId, emoji }),

  // Pins
  pinMessage: (communityId: string, channelId: string, messageId: string) =>
    invoke<void>("pin_message", { communityId, channelId, messageId }),
  unpinMessage: (communityId: string, channelId: string, messageId: string) =>
    invoke<void>("unpin_message", { communityId, channelId, messageId }),
  getChannelPins: (communityId: string, channelId: string) =>
    invoke<{ messageId: string; channelId: string; pinnedBy: string; pinnedAt: number }[]>(
      "get_channel_pins",
      { communityId, channelId },
    ),

  sendChannelTyping: (communityId: string, channelId: string) =>
    invoke<void>("send_channel_typing", { communityId, channelId }),

  updateCommunityPresence: (communityId: string, status: string, gameName?: string, gameId?: number, elapsedSeconds?: number, serverAddress?: string) =>
    invoke<void>("update_community_presence", { communityId, status, gameName, gameId, elapsedSeconds, serverAddress }),

  // Audit log
  getAuditLog: (communityId: string, beforeTimestamp?: number, limit: number = 50) =>
    invoke<{ action: string; actorPseudonym: string; target: string | null; details: string | null; timestamp: number }[]>(
      "get_audit_log",
      { communityId, beforeTimestamp, limit },
    ),

  // Categories
  createCategory: (communityId: string, name: string) =>
    invoke<{ categoryId: string }>("create_category", { communityId, name }),
  deleteCategory: (communityId: string, categoryId: string) =>
    invoke<void>("delete_category", { communityId, categoryId }),
  renameCategory: (communityId: string, categoryId: string, newName: string) =>
    invoke<void>("rename_category", { communityId, categoryId, newName }),
  moveChannel: (communityId: string, channelId: string, categoryId: string | null) =>
    invoke<void>("move_channel", { communityId, channelId, categoryId }),
  reorderCategories: (communityId: string, categoryIds: string[]) =>
    invoke<void>("reorder_categories", { communityId, categoryIds }),
  setChannelTopic: (communityId: string, channelId: string, topic: string) =>
    invoke<void>("set_channel_topic", { communityId, channelId, topic }),
  reorderChannels: (communityId: string, channelIds: string[]) =>
    invoke<void>("reorder_channels", { communityId, channelIds }),

  // Community invites
  createCommunityInvite: (communityId: string, maxUses?: number, expiresInSeconds?: number) =>
    invoke<{ code: string; signature: string }>("create_community_invite", { communityId, maxUses: maxUses ?? null, expiresInSeconds: expiresInSeconds ?? null }),
  revokeCommunityInvite: (communityId: string, code: string) =>
    invoke<void>("revoke_community_invite", { communityId, code }),
  listCommunityInvites: (communityId: string) =>
    invoke<{ code: string; createdBy: string; maxUses: number | null; uses: number; expiresAt: number | null; createdAt: number }[]>(
      "list_community_invites", { communityId }
    ),

  // Roles
  getRoles: (communityId: string) =>
    invoke<{ id: number; name: string; color: number; permissions: number; position: number; hoist: boolean; mentionable: boolean }[]>(
      "get_roles", { communityId },
    ),
  createRole: (communityId: string, name: string, color: number, permissions: string, hoist: boolean, mentionable: boolean) =>
    invoke<number>("create_role", { communityId, name, color, permissions, hoist, mentionable }),
  editRole: (communityId: string, roleId: number, name: string | null, color: number | null, permissions: string | null, position: number | null, hoist: boolean | null, mentionable: boolean | null) =>
    invoke<void>("edit_role", { communityId, roleId, name, color, permissions, position, hoist, mentionable }),
  deleteRole: (communityId: string, roleId: number) =>
    invoke<void>("delete_role", { communityId, roleId }),
  assignRole: (communityId: string, pseudonymKey: string, roleId: number) =>
    invoke<void>("assign_role", { communityId, pseudonymKey, roleId }),
  unassignRole: (communityId: string, pseudonymKey: string, roleId: number) =>
    invoke<void>("unassign_role", { communityId, pseudonymKey, roleId }),
  timeoutMember: (communityId: string, pseudonymKey: string, durationSeconds: number, reason: string | null) =>
    invoke<void>("timeout_member", { communityId, pseudonymKey, durationSeconds, reason }),
  removeTimeout: (communityId: string, pseudonymKey: string) =>
    invoke<void>("remove_timeout", { communityId, pseudonymKey }),
  setChannelOverwrite: (communityId: string, channelId: string, targetType: string, targetId: string, allow: number, deny: number) =>
    invoke<void>("set_channel_overwrite", { communityId, channelId, targetType, targetId, allow, deny }),
  deleteChannelOverwrite: (communityId: string, channelId: string, targetType: string, targetId: string) =>
    invoke<void>("delete_channel_overwrite", { communityId, channelId, targetType, targetId }),
  setSlowmode: (communityId: string, channelId: string, seconds: number) =>
    invoke<void>("set_slowmode", { communityId, channelId, seconds }),

  // Threads
  createThread: (communityId: string, channelId: string, name: string, starterMessageId: string) =>
    invoke<string>("create_thread", { communityId, channelId, name, starterMessageId }),
  getChannelThreads: (communityId: string, channelId: string) =>
    invoke<{ id: string; channelId: string; name: string; starterMessageId: string; creatorPseudonym: string; createdAt: number; archived: boolean; autoArchiveSeconds: number; lastMessageAt: number; messageCount: number }[]>(
      "get_channel_threads", { communityId, channelId }),
  sendThreadMessage: (communityId: string, threadId: string, body: string) =>
    invoke<void>("send_thread_message", { communityId, threadId, body }),
  getThreadMessages: (communityId: string, threadId: string, limit: number, beforeTimestamp?: number) =>
    invoke<Message[]>("get_thread_messages", { communityId, threadId, limit, beforeTimestamp: beforeTimestamp ?? null }),
  archiveThread: (communityId: string, threadId: string) =>
    invoke<void>("archive_thread", { communityId, threadId }),
  unarchiveThread: (communityId: string, threadId: string) =>
    invoke<void>("unarchive_thread", { communityId, threadId }),

  // Community Events
  createEvent: (communityId: string, title: string, description: string, startTime: number, endTime?: number, channelId?: string, maxAttendees?: number) =>
    invoke<string>("create_event", { communityId, title, description, startTime, endTime: endTime ?? null, channelId: channelId ?? null, maxAttendees: maxAttendees ?? null }),
  editEvent: (communityId: string, eventId: string, title?: string, description?: string, startTime?: number, endTime?: number, channelId?: string, maxAttendees?: number) =>
    invoke<void>("edit_event", { communityId, eventId, title: title ?? null, description: description ?? null, startTime: startTime ?? null, endTime: endTime ?? null, channelId: channelId ?? null, maxAttendees: maxAttendees ?? null }),
  deleteEvent: (communityId: string, eventId: string) =>
    invoke<void>("delete_event", { communityId, eventId }),
  cancelEvent: (communityId: string, eventId: string) =>
    invoke<void>("cancel_event", { communityId, eventId }),
  rsvpEvent: (communityId: string, eventId: string, status: string) =>
    invoke<void>("rsvp_event", { communityId, eventId, status }),
  getEvents: (communityId: string) =>
    invoke<{ id: string; title: string; description: string; creatorPseudonym: string; startTime: number; endTime: number | null; channelId: string | null; maxAttendees: number | null; createdAt: number; status: string; rsvps: { pseudonymKey: string; status: string }[] }[]>(
      "get_events", { communityId },
    ),

  // Game Servers
  addGameServer: (communityId: string, gameId: string, label: string, address: string) =>
    invoke<string>("add_game_server", { communityId, gameId, label, address }),
  removeGameServer: (communityId: string, serverId: string) =>
    invoke<void>("remove_game_server", { communityId, serverId }),
  getGameServers: (communityId: string) =>
    invoke<{ id: string; gameId: string; label: string; address: string; addedBy: string; createdAt: number }[]>(
      "get_game_servers", { communityId }),

  // Unread tracking
  markChannelRead: (communityId: string, channelId: string, lastMessageId: string) =>
    invoke<void>("mark_channel_read", { communityId, channelId, lastMessageId }),
  getUnreadCounts: (communityId: string) =>
    invoke<{ channelId: string; unreadCount: number }[]>("get_unread_counts", { communityId }),

  // Voice
  joinVoiceChannel: (channelId: string) =>
    invoke<void>("join_voice_channel", { channelId }),
  leaveVoice: () => invoke<void>("leave_voice"),
  setMute: (muted: boolean) => invoke<void>("set_mute", { muted }),
  setDeafen: (deafened: boolean) => invoke<void>("set_deafen", { deafened }),
  listAudioDevices: () => invoke<AudioDevices>("list_audio_devices"),
  setAudioDevices: (inputDevice?: string, outputDevice?: string) =>
    invoke<void>("set_audio_devices", {
      inputDevice: inputDevice ?? null,
      outputDevice: outputDevice ?? null,
    }),

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
  getNetworkStatus: () => invoke<NetworkStatus>("get_network_status"),

  // Onboarding & Welcome Screen
  getOnboardingConfig: (communityId: string) =>
    invoke<OnboardingConfig>("get_onboarding_config", { communityId }),
  setOnboardingConfig: (communityId: string, config: OnboardingConfig) =>
    invoke<void>("set_onboarding_config", { communityId, config }),
  getWelcomeScreen: (communityId: string) =>
    invoke<WelcomeScreen>("get_welcome_screen", { communityId }),
  setWelcomeScreen: (communityId: string, screen: WelcomeScreen) =>
    invoke<void>("set_welcome_screen", { communityId, screen }),
  submitOnboardingAnswers: (communityId: string, answers: OnboardingAnswer[]) =>
    invoke<void>("submit_onboarding_answers", { communityId, answers }),
};

export function avatarDataUrl(base64: string | null | undefined): string | undefined {
  if (!base64) return undefined;
  return `data:image/webp;base64,${base64}`;
}
