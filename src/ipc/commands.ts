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

export interface Preferences {
  notificationsEnabled: boolean;
  notificationSound: boolean;
  startMinimized: boolean;
  autoStart: boolean;
  gameDetectionEnabled: boolean;
  gameScanIntervalSecs: number;
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
    }[]>("get_community_details"),
  getCommunityMembers: (communityId: string) =>
    invoke<{ publicKey: string; displayName: string; role: string; status: string }[]>(
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
  removeCommunityMember: (communityId: string, publicKey: string) =>
    invoke<void>("remove_community_member", { communityId, publicKey }),
  updateMemberRole: (communityId: string, publicKey: string, role: string) =>
    invoke<void>("update_member_role", { communityId, publicKey, role }),
  leaveCommunity: (communityId: string) =>
    invoke<void>("leave_community", { communityId }),

  // Voice
  joinVoiceChannel: (channelId: string) =>
    invoke<void>("join_voice_channel", { channelId }),
  leaveVoice: () => invoke<void>("leave_voice"),
  setMute: (muted: boolean) => invoke<void>("set_mute", { muted }),
  setDeafen: (deafened: boolean) => invoke<void>("set_deafen", { deafened }),

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
  openSettingsWindow: () => invoke<void>("open_settings_window"),
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
