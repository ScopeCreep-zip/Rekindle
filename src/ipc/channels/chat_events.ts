/**
 * Events on the `chat-event` channel.
 *
 * This channel is mid-migration and carries **two** shapes:
 *
 * 1. {@link LegacyChatEvent} — the desktop's old `{ type, data }`
 *    envelope. Only `messageReceived` remains, for community **channel**
 *    messages. See `src-tauri/src/channels/chat_channel.rs` for why it
 *    survived: Tier 1 models a channel message as
 *    `ChannelMessageEvent::New`, which needs `community` and `sequence`
 *    that the desktop's five emitters do not have.
 *
 * 2. {@link ChatSubscriptionEvent} — the daemon vocabulary the CLI also
 *    consumes, externally tagged (`{ familyName: { variantName: {...} } }`).
 *    Calls, friend lifecycle, DMs, typing and acks all arrive this way.
 *
 * {@link isLegacy} discriminates the two.
 */

/** DM / conversation message from the daemon vocabulary. */
export interface DirectMessageReceived {
  peerKey: string;
  timestamp: number;
  senderName: string | null;
  body: string | null;
  decryptionFailed: boolean;
  automodBlurred: boolean;
  conversationId: string;
  serverMessageId: string | null;
  replyToId: string | null;
}

export type ChannelMessageEvent =
  | { directMessageReceived: DirectMessageReceived }
  | { directMessageAcknowledged: { messageId: number } }
  | {
      directConversationInvited: {
        from: string;
        recordKey: string;
        initiatorPseudonym: string;
        isGroup: boolean;
      };
    }
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
  | { dm: { peerKey: string } };

export type TypingEvent =
  | { started: { context: TypingContext; who: string } }
  | { stopped: { context: TypingContext; who: string } };

export type FriendEvent =
  | { requestReceived: { fromKey: string; displayName: string; message: string } }
  | { requestAcknowledged: { peerKey: string } }
  | {
      accepted: {
        peerKey: string;
        dmLogKey: string | null;
        displayName: string | null;
      };
    }
  | { rejected: { peerKey: string } }
  | {
      added: {
        peerKey: string;
        displayName: string;
        friendshipState: string;
      };
    }
  | { removed: { peerKey: string } }
  | { removeAcknowledged: { peerKey: string } }
  | { profileKeyRotated: { peerKey: string; newProfileDhtKey: string } };

/** Peer on the other end of a direct call; absent for a group call. */
export interface DirectCallInfo {
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
  | { connected: { callId: string; direct: DirectCallInfo | null } }
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
  | { participantLeft: { callId: string; participantPubkey: string; reason: string } };

/** The daemon-vocabulary families delivered on this channel. */
export type ChatSubscriptionEvent =
  | { channelMessage: ChannelMessageEvent }
  | { typing: TypingEvent }
  | { friend: FriendEvent }
  | { call: CallEvent };

/** The desktop's remaining `{ type, data }` event. */
export type LegacyChatEvent = {
  type: "messageReceived";
  data: {
    from: string;
    body: string;
    decryptionFailed?: boolean;
    automodBlurred?: boolean;
    timestamp: number;
    conversationId: string;
    serverMessageId?: string;
    replyToId?: string;
    senderDisplayName?: string;
  };
};

export type ChatEvent = LegacyChatEvent | ChatSubscriptionEvent;

/** Whether this is the old `{ type, data }` envelope. */
export function isLegacy(event: ChatEvent): event is LegacyChatEvent {
  return "type" in event;
}
