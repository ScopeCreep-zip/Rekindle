import { createStore } from "solid-js/store";
import type { DeliveryRetry } from "../ipc/channels/chat_events";
import type { GameInfo } from "../ipc/commands/dto";
import type { SendState } from "../ipc/commands/types";
import type { UserStatus } from "./auth.store";

type FriendshipState = "pendingOut" | "accepted";

export interface Friend {
  publicKey: string;
  displayName: string;
  nickname: string | null;
  status: UserStatus;
  statusMessage: string | null;
  gameInfo: GameInfo | null;
  /** The group label the buddy list shows them under (the Host's). */
  group: string;
  /** Our group they are in, the id the group commands take; null when they
   *  are in none (`group` is then the Host's default label). */
  groupId: number | null;
  unreadCount: number;
  lastSeenAt: number | null;
  voiceChannel: string | null;
  friendshipState: FriendshipState;
  /** Our friend request's delivery, while it is pending out (plan E2.5). */
  requestSendState?: SendState;
  /** Our friend request goes unanswered and the Host keeps trying (plan SR2), else null. */
  requestRetry?: DeliveryRetry | null;
  /** Why the secure session could not be set up, while it is failed (C7.25). */
  handshakeError: string | null;
  /** Our first contact is being resent from a fresh key (plan E2.4, B6). */
  handshakeRetrying: boolean;
  /// The peer's route does not answer (plan RH1r-c S4).
  unreachable: boolean;
  /** Their keys are not signed by the identity we know: a trust alert on
   *  the row, and the chat refuses to send (plan E2.4, B3). */
  identityPinMismatch: boolean;
  /** The Host's reason they cannot join a new group DM yet (GC3). */
  groupDmBlock: string | null;
}

interface PendingRequest {
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

interface FriendsState {
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
