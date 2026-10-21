import { createStore } from "solid-js/store";
import type { UserStatus } from "./auth.store";
import type { GameInfo } from "../ipc/commands/dto";

export type { GameInfo } from "../ipc/commands/dto";

export type FriendshipState = "pendingOut" | "accepted";

export interface Friend {
  publicKey: string;
  displayName: string;
  nickname: string | null;
  status: UserStatus;
  statusMessage: string | null;
  gameInfo: GameInfo | null;
  /** The group label the buddy list shows them under (the Host's). */
  group: string;
  /** They are in no group of ours: `group` is the Host's default label. */
  inDefaultGroup: boolean;
  unreadCount: number;
  lastSeenAt: number | null;
  voiceChannel: string | null;
  friendshipState: FriendshipState;
  /** Why the secure session could not be set up, while it is failed (C7.25). */
  handshakeError: string | null;
  /** Our first contact is being resent from a fresh key (plan E2.4, B6). */
  handshakeRetrying: boolean;
  /** Their keys are not signed by the identity we know: a trust alert on
   *  the row, and the chat refuses to send (plan E2.4, B3). */
  identityPinMismatch: boolean;
}

export interface PendingRequest {
  publicKey: string;
  displayName: string;
  message: string;
}

export interface OutgoingInvite {
  inviteId: string;
  url: string;
  createdAt: number;
  expiresAt: number;
  status: string;
  acceptedBy: string | null;
}

export interface FriendsState {
  friends: Record<string, Friend>;
  pendingRequests: PendingRequest[];
  outgoingInvites: OutgoingInvite[];
  showAddFriend: boolean;
  showNewChat: boolean;
}

const [friendsState, setFriendsState] = createStore<FriendsState>({
  friends: {},
  pendingRequests: [],
  outgoingInvites: [],
  showAddFriend: false,
  showNewChat: false,
});

export { friendsState, setFriendsState };
