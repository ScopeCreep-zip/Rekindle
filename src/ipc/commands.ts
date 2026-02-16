import { invoke } from "./invoke";

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
}

export interface GameStatus {
  gameId: number;
  gameName: string;
  serverInfo: string | null;
  elapsedSeconds: number;
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

  // Community
  getCommunities: () =>
    invoke<{ id: string; name: string; channelCount: number }[]>("get_communities"),
  getCommunityDetails: () =>
    invoke<{
      id: string;
      name: string;
      description: string | null;
      channels: { id: string; name: string; channelType: string; unreadCount: number }[];
      myRole: string | null;
      myRoleIds: number[];
      roles: { id: number; name: string; color: number; permissions: number; position: number; hoist: boolean; mentionable: boolean }[];
      myPseudonymKey: string | null;
      mekGeneration: number;
      isHosted: boolean;
    }[]>("get_community_details"),
  getCommunityMembers: (communityId: string) =>
    invoke<{ pseudonymKey: string; displayName: string; roleIds: number[]; displayRole: string; status: string; timeoutUntil: number | null }[]>(
      "get_community_members", { communityId },
    ),
  createCommunity: (name: string) =>
    invoke<string>("create_community", { name }),
  joinCommunity: (communityId: string) =>
    invoke<void>("join_community", { communityId }),
  createChannel: (communityId: string, name: string, channelType: string) =>
    invoke<string>("create_channel", { communityId, name, channelType }),
  sendChannelMessage: (channelId: string, body: string) =>
    invoke<void>("send_channel_message", { channelId, body }),
  getChannelMessages: (channelId: string, limit: number) =>
    invoke<Message[]>("get_channel_messages", { channelId, limit }),
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

  // Roles
  getRoles: (communityId: string) =>
    invoke<{ id: number; name: string; color: number; permissions: number; position: number; hoist: boolean; mentionable: boolean }[]>(
      "get_roles", { communityId },
    ),
  createRole: (communityId: string, name: string, color: number, permissions: number, hoist: boolean, mentionable: boolean) =>
    invoke<number>("create_role", { communityId, name, color, permissions, hoist, mentionable }),
  editRole: (communityId: string, roleId: number, name: string | null, color: number | null, permissions: number | null, position: number | null, hoist: boolean | null, mentionable: boolean | null) =>
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
};

export function avatarDataUrl(base64: string | null | undefined): string | undefined {
  if (!base64) return undefined;
  return `data:image/webp;base64,${base64}`;
}
