import { invoke } from "../invoke";
import type {
  FriendInfo, IdentitySummary, LoginResult, Message,
} from "./types";

export const accountCommands = {
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
  sendMessage: (to: string, body: string, idempotencyKey?: string) =>
    invoke<void>("send_message", {
      to,
      body,
      // Phase 8 — idempotency key dedupes click-spam. Caller may pass
      // their own (e.g. a retry uses the SAME key to short-circuit);
      // default is a fresh UUID per call.
      idempotencyKey: idempotencyKey ?? crypto.randomUUID(),
    }),
  sendTyping: (peerId: string, typing: boolean) =>
    invoke<void>("send_typing", { peerId, typing }),
  getMessageHistory: (peerId: string, limit: number) =>
    invoke<Message[]>("get_message_history", { peerId, limit }),
  markRead: (peerId: string) => invoke<void>("mark_read", { peerId }),

  // Friends
  addFriend: (
    publicKey: string,
    displayName: string,
    message: string,
    idempotencyKey?: string,
  ) =>
    invoke<void>("add_friend", {
      publicKey,
      displayName,
      message,
      // Phase 8 — idempotency key dedupes click-spam. Default is a
      // fresh UUID per call; retries should pass the SAME key to
      // short-circuit to the cached result.
      idempotencyKey: idempotencyKey ?? crypto.randomUUID(),
    }),
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

  /** Set (or clear, with null) the local alias shown for one friend.
   *  Distinct from `setNickname`, which changes the user's own display
   *  name — this is the per-friend alias the buddy list renders. */
  setFriendNickname: (publicKey: string, nickname: string | null) =>
    invoke<void>("set_friend_nickname", { publicKey, nickname }),
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

};
