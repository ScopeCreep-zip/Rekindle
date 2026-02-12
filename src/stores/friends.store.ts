import { createStore } from "solid-js/store";
import type { UserStatus } from "./auth.store";

export interface GameInfo {
  gameName: string;
  gameId: number | null;
  startedAt: number | null;
}

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
}

export interface PendingRequest {
  publicKey: string;
  displayName: string;
  message: string;
}

export interface ContextMenuState {
  x: number;
  y: number;
  publicKey: string;
}

export interface FriendsState {
  friends: Record<string, Friend>;
  pendingRequests: PendingRequest[];
  contextMenu: ContextMenuState | null;
  showAddFriend: boolean;
  showNewChat: boolean;
}

const [friendsState, setFriendsState] = createStore<FriendsState>({
  friends: {},
  pendingRequests: [],
  contextMenu: null,
  showAddFriend: false,
  showNewChat: false,
});

export { friendsState, setFriendsState };
