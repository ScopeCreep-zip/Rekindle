/**
 * Events on the `chat-event` channel: the daemon vocabulary the CLI also
 * consumes, externally tagged (`{ familyName: { variantName: {...} } }`).
 * Calls, friend lifecycle, DMs, typing and acks all arrive this way.
 */

import type { DmMessage } from "../commands/dto_dm";
import type { SendState } from "../commands/types";
import type { LobbyView } from "../commands/dto_lobby";

/** DM / conversation message from the daemon vocabulary. */
interface DirectMessageReceived {
  peerKey: string;
  timestamp: number;
  senderName: string | null;
  body: string | null;
  decryptionFailed: boolean;
  automodBlurred: boolean;
  conversationId: string;
  serverMessageId: string | null;
  replyToId: string | null;
  isOwn: boolean;
}

/** Our own direct message exactly as stored, once committed (plan E6.11):
 *  every window of the conversation upserts it by `message.messageId`. Never
 *  a peer's message, so it never notifies or counts unread. */
interface DirectMessageSent {
  conversationId: string;
  replyToId: string | null;
  message: DmMessage;
}

/** A direct message we sent changed delivery state (the outbox's one
 *  vocabulary: queued, sent, delivered, read, failed). `clientId` is its
 *  client id (`DmMessage.messageId`, `serverMessageId` of a received one). */
export interface DmSendStatus {
  conversationId: string;
  clientId: string;
  state: SendState;
  /** Set while the send goes unanswered and the Host keeps trying (plan SR2):
   *  each unanswered attempt publishes it; delivery or failure clears it. */
  retry: DeliveryRetry | null;
}

/** `DeliveryRetry` (plan SR2): attempts so far and when the next one goes (ms since epoch). */
export interface DeliveryRetry {
  attempts: number;
  nextAttemptAt: number;
}

/** The peer's DM chain retired entries we never fetched: their stream to
 *  us is gone up to and including `throughSeq` (plan E2.5). */
export interface DmHistoryGap {
  peerKey: string;
  throughSeq: number;
}

/** The DM or group DM an event names (`ConversationContext`, plan E7.1). */
export type ConversationContext = { dm: { peerKey: string } } | { groupDm: { groupId: string } };

/** The peer's key, or the group's id. */
export function conversationId(context: ConversationContext): string {
  return "dm" in context ? context.dm.peerKey : context.groupDm.groupId;
}

/** A conversation's disappearing timer changed: `setBy` (our key or a
 *  member's) set it to `seconds` (0 = off). */
export interface DisappearingTimerChanged {
  context: ConversationContext;
  seconds: number;
  setBy: string;
  /** The stored notice row's client id, which `messagesExpired` names. */
  clientId: string;
  /** When it was set (ms), the row's timestamp. */
  timestamp: number;
}

/** The Host deleted these messages at their `expireAt`. */
interface MessagesExpired {
  context: ConversationContext;
  clientIds: string[];
}

export type ChannelMessageEvent =
  | { directMessageReceived: DirectMessageReceived }
  | { directMessageSent: DirectMessageSent }
  | { disappearingTimerChanged: DisappearingTimerChanged }
  | { messagesExpired: MessagesExpired }
  | { dmSendStatus: DmSendStatus }
  | { dmHistoryGap: DmHistoryGap }
  | {
      conversationFocusRequested: {
        peerKey: string;
        displayName: string;
        reason: string;
      };
    }
  | { new: Record<string, unknown> }
  | { edited: Record<string, unknown> }
  | { deleted: Record<string, unknown> };

export type TypingContext =
  | { channel: { community: string; channel: string } }
  | { dm: { peerKey: string } }
  | { groupDm: { groupId: string } };

/** A group member as an event names them (our nickname or fingerprint,
 *  never a name their payload carries). */
export interface GroupDmMember {
  publicKey: string;
  displayName: string;
}

/** `subscription_events::GroupDmEvent` (plan E2.6). */
export type GroupDmEvent =
  | {
      invited: {
        groupId: string;
        name: string;
        inviter: GroupDmMember;
        members: GroupDmMember[];
      };
    }
  | { joined: { groupId: string; member: GroupDmMember } }
  | { left: { groupId: string; member: GroupDmMember } }
  | { keyRotated: { groupId: string; member: GroupDmMember; epoch: number } }
  | { renamed: { groupId: string; name: string } }
  /** The group's lobby changed (plan E7.2 G6); null when there is none. */
  | { lobby: { groupId: string; lobby: LobbyView | null } }
  /** A group send's state; `retry` while unanswered attempts are retried (plan SR2b). */
  | {
      sendStatus: {
        groupId: string;
        clientId: string;
        state: SendState;
        retry: DeliveryRetry | null;
      };
    }
  | {
      message: {
        groupId: string;
        messageId: string;
        sender: GroupDmMember;
        body: string;
        timestamp: number;
        replyTo: string | null;
      };
    }
  /** Our own message exactly as stored, once committed (plan E6.11): every
   *  window of the group upserts it by `message.messageId`. Never a member's
   *  message, so it never notifies or counts unread. */
  | { messageSent: { groupId: string; replyToId: string | null; message: DmMessage } }
  | { historyGap: { groupId: string; member: GroupDmMember; throughSeq: number } };

type TypingEvent =
  // `name`: the typist as the Host names them (your nickname, else their name, else fingerprint).
  | { started: { context: TypingContext; who: string; name: string } }
  | { stopped: { context: TypingContext; who: string } };

export type FriendEvent =
  | { requestReceived: { fromKey: string; displayName: string; message: string } }
  | { requestAcknowledged: { peerKey: string } }
  | { accepted: { peerKey: string; displayName: string | null } }
  | { rejected: { peerKey: string } }
  | {
      added: {
        peerKey: string;
        displayName: string;
        friendshipState: string;
        /** The group the buddy list shows them under (the backend's). */
        groupLabel: string;
      };
    }
  | { removed: { peerKey: string } }
  | { removeAcknowledged: { peerKey: string } }
  | { profileKeyRotated: { peerKey: string; newProfileDhtKey: string } }
  | { nicknameChanged: { peerKey: string; nickname: string | null } }
  | { displayNameChanged: { peerKey: string; displayName: string } }
  | {
      groupChanged: {
        peerKey: string;
        groupId: number | null;
        group: string | null;
        groupLabel: string;
      };
    }
  | { keyChanged: { peerKey: string; newKey: string } }
  | { handshakeFailed: { peerKey: string; reason: string } }
  | { handshakeRetrying: { peerKey: string } }
  | { reachability: { peerKey: string; reachable: boolean } }
  | { identityPinMismatch: { peerKey: string; displayName: string } }
  | { identityPinRestored: { peerKey: string } }
  | { verificationChanged: { peerKey: string; verified: boolean } }
  | { blockChanged: { peerKey: string; blocked: boolean } }
  /** An accepted friend invites us to their game server (plan E7.2);
   *  `server` is the Host-parsed host:port. Joining is reviewed. */
  | { gameInvite: { peerKey: string; gameId: number; server: string } }
  | { creatorInvited: { peerKey: string; anchor: string; communityName: string } }
  | { creatorInviteWithdrawn: { peerKey: string; anchor: string } }
  | { coCreatorStatus: { peerKey: string; anchor: string; status: string } }
  /** Our co-creation of `anchor` ended: `created` when its genesis is
   *  written, otherwise it failed (plan E3.2 S7s.3g). */
  | { coCreationFinished: { anchor: string; created: boolean } };

/** Peer on the other end of a direct call; absent for a group call. */
interface DirectCallInfo {
  kind: string;
  peerKey: string;
  peerDisplayName: string;
  expectedLocalCamera: boolean;
}

export type CallEvent =
  | {
      incoming: {
        callId: string;
        from: string;
        displayName: string;
        kind: string;
        /** Everyone invited, for a group call. Empty for a 1:1. */
        participants: string[];
        isGroup: boolean;
        /** The group the call is for (GV2); null for a buddy-list call or a 1:1. */
        groupId: string | null;
        expiresAtMs: number;
      };
    }
  | { ringing: { callId: string } }
  | {
      started: {
        callId: string;
        kind: string;
        peerKey: string;
        peerDisplayName: string;
        expiresAtMs: number;
      };
    }
  | {
      /** We started a group call; it rings out until the first accept. */
      groupStarted: {
        callId: string;
        kind: string;
        participants: string[];
        groupId: string | null;
        expiresAtMs: number;
      };
    }
  | { connected: { callId: string; direct: DirectCallInfo | null; groupId: string | null } }
  | { declined: { callId: string; reason: string } }
  | { missed: { callId: string; from: string } }
  | { timedOut: { callId: string } }
  /** Covers 1:1 and group alike — the two had identical fields. */
  | { ended: { callId: string; reason: string } }
  | {
      mediaStateChanged: {
        callId: string;
        audio: boolean;
        video: boolean;
        screen: boolean;
        timestampMs: number;
      };
    }
  | {
      reactionReceived: {
        callId: string;
        sender: string;
        emoji: string;
        timestampMs: number;
      };
    }
  | { participantJoined: { callId: string; participantPubkey: string } }
  | { participantLeft: { callId: string; participantPubkey: string; reason: string } }
  /** Our own mute changed; names no call (lead decision 58). */
  | { selfMuteChanged: { muted: boolean; timestampMs: number } };

/** What an unread count belongs to (`subscription_events::UnreadContext`). */
type UnreadContext =
  | { channel: { community: string; channel: string } }
  | { dm: { peerKey: string } }
  | "friendRequests";

/** The Host's unread count for one context, rendered as is. */
export interface UnreadChanged {
  context: UnreadContext;
  count: number;
}

/** The daemon-vocabulary families delivered on this channel. */
type ChatSubscriptionEvent =
  | { channelMessage: ChannelMessageEvent }
  | { typing: TypingEvent }
  | { friend: FriendEvent }
  | { call: CallEvent }
  | { groupDm: GroupDmEvent }
  | { unreadChanged: UnreadChanged };

/** Every event on the `chat-event` channel. */
export type ChatEvent = ChatSubscriptionEvent;
