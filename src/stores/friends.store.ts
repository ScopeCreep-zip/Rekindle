import { createStore } from "solid-js/store";
import type { UserStatus } from "./auth.store";
import type { GameInfo } from "./types";

export type { GameInfo } from "./types";

export type FriendshipState = "pendingOut" | "accepted";

export interface Friend {
  publicKey: string;
  displayName: string;
  nickname: string | null;
  status: UserStatus;
  statusMessage: string | null;
  gameInfo: GameInfo | null;
  group: string;
  unreadCount: number;
  lastSeenAt: number | null;
  voiceChannel: string | null;
  friendshipState: FriendshipState;
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
