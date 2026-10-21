import { hostInvoke } from "../invoke";
import type { FriendInfo } from "./types";
import type {
  BlockedUser,
  DmMessage,
  IdentityShown,
  IdentitySummary,
  IdentityView,
  FriendInviteCreated,
  OutgoingInvite,
  PendingRequest,
  SafetyNumberView,
  FriendCodeView,
} from "./dto";

export const accountCommands = {
  // Identity: Host calls (plan E1.1). A blank name is no name: the Host
  // names the identity `User_` and its key's first characters.
  createIdentity: (passphrase: string, displayName?: string) =>
    hostInvoke<IdentityView>("create_identity", {
      passphrase,
      displayName: displayName?.trim() ? displayName.trim() : null,
    }),
  login: (publicKey: string, passphrase: string) =>
    hostInvoke<IdentityView>("login", { publicKey, passphrase }),
  getIdentity: () => hostInvoke<IdentityShown | null>("get_identity"),
  logout: () => hostInvoke<unknown>("logout"),
  listIdentities: () => hostInvoke<IdentitySummary[]>("list_identities"),
  /** Protected: the Host asks for the identity's passphrase in the review. */
  deleteIdentity: (publicKey: string) => hostInvoke<unknown>("delete_identity", { publicKey }),

  // Direct messages and friends: Host calls (plan E2.1). The windows learn
  // their effects from the `Friend`/`ChannelMessage` events.
  /** Resolves with the stored message (`sendState: "sent"`); a failed send
   *  rejects, and its stored row reads `failed` in the history. */
  sendMessage: (to: string, body: string, replyTo: string | null = null) =>
    hostInvoke<DmMessage>("send_message", { to, body, replyTo }),
  /** Sends our failed message again by its client id; the Host re-frames
   *  the stored row, stores no new one, and publishes its new state. */
  resendMessage: (messageId: string) =>
    hostInvoke<DmMessage>("resend_message", { messageId }),
  sendTyping: (peerId: string, typing: boolean) =>
    hostInvoke<unknown>("send_typing", { peerId, typing }),
  getMessageHistory: (peerId: string, limit: number, before?: number) =>
    hostInvoke<DmMessage[]>("get_message_history", { peerId, limit, before: before ?? null }),
  markRead: (peerId: string) => hostInvoke<unknown>("mark_read", { peerId }),

  addFriendFromCode: (code: string, message: string) =>
    hostInvoke<unknown>("add_friend_from_code", { code, message }),
  getFriendCode: () => hostInvoke<FriendCodeView>("get_friend_code"),
  /** Reviewed by the Host (Consequential): codes already shared stop
   *  working; resolves with the new code. */
  rotateFriendToken: () => hostInvoke<FriendCodeView>("rotate_friend_token"),
  removeFriend: (publicKey: string) => hostInvoke<unknown>("remove_friend", { publicKey }),
  acceptRequest: (publicKey: string) => hostInvoke<unknown>("accept_request", { publicKey }),
  rejectRequest: (publicKey: string) => hostInvoke<unknown>("reject_request", { publicKey }),
  getFriends: () => hostInvoke<FriendInfo[]>("get_friends"),
  getPendingRequests: () => hostInvoke<PendingRequest[]>("get_pending_requests"),
  createFriendGroup: (name: string) =>
    hostInvoke<{ groupId: number }>("create_friend_group", { name }),
  renameFriendGroup: (groupId: number, name: string) =>
    hostInvoke<unknown>("rename_friend_group", { groupId, name }),
  deleteFriendGroup: (groupId: number) =>
    hostInvoke<unknown>("delete_friend_group", { groupId }),

  /** Set (or clear, with null) the local alias shown for one friend.
   *  Distinct from `setNickname`, which changes the user's own display
   *  name — this is the per-friend alias the buddy list renders. */
  setFriendNickname: (publicKey: string, nickname: string | null) =>
    hostInvoke<unknown>("set_friend_nickname", { publicKey, nickname }),
  moveFriendToGroup: (publicKey: string, groupId: number | null) =>
    hostInvoke<unknown>("move_friend_to_group", { publicKey, groupId }),
  generateInvite: () => hostInvoke<FriendInviteCreated>("generate_invite"),
  /** Reviewed with the inviter's key fingerprint before it runs. */
  addFriendFromInvite: (inviteString: string) =>
    hostInvoke<unknown>("add_friend_from_invite", { inviteString }),
  cancelInvite: (inviteId: string) => hostInvoke<unknown>("cancel_invite", { inviteId }),
  getOutgoingInvites: () => hostInvoke<OutgoingInvite[]>("get_outgoing_invites"),
  blockUser: (publicKey: string) => hostInvoke<unknown>("block_user", { publicKey }),
  unblockUser: (publicKey: string) => hostInvoke<unknown>("unblock_user", { publicKey }),
  getBlockedUsers: () => hostInvoke<BlockedUser[]>("get_blocked_users"),
  cancelRequest: (publicKey: string) => hostInvoke<unknown>("cancel_request", { publicKey }),
  retryHandshake: (publicKey: string) => hostInvoke<unknown>("retry_handshake", { publicKey }),
  /** TrustChanging: the review shows whose session it is; the friend
   *  compares safety numbers before accepting. */
  resetSignalSession: (peerPublicKey: string) =>
    hostInvoke<unknown>("reset_signal_session", { peerPublicKey }),

  // Safety numbers and verification (plan E2.3).
  getSafetyNumber: (publicKey: string) =>
    hostInvoke<SafetyNumberView>("get_safety_number", { publicKey }),
  /** TrustChanging: the review states the safety number the user compared. */
  markIdentityVerified: (publicKey: string) =>
    hostInvoke<unknown>("mark_identity_verified", { publicKey }),
  /** TrustChanging: the review states the safety number. */
  clearIdentityVerification: (publicKey: string) =>
    hostInvoke<unknown>("clear_identity_verification", { publicKey }),
};
