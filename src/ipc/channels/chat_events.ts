export type ChatEvent =
  | {
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
    }
  | { type: "typingIndicator"; data: { from: string; typing: boolean } }
  | { type: "messageAck"; data: { messageId: number } }
  | {
      type: "friendRequest";
      data: { from: string; displayName: string; message: string };
    }
  | {
      type: "friendRequestAccepted";
      data: { from: string; displayName: string };
    }
  | {
      type: "friendAdded";
      data: { publicKey: string; displayName: string; friendshipState: string };
    }
  | { type: "friendRequestRejected"; data: { from: string } }
  | { type: "friendRemoved"; data: { publicKey: string } }
  | { type: "friendRequestDelivered"; data: { to: string } }
  | {
      type: "directMessageInvite";
      data: {
        from: string;
        recordKey: string;
        initiatorPseudonym: string;
        isGroup: boolean;
      };
    }
  // Plan §Failure 5 — direct call signalling. The backend is the
  // single source of truth for call state; the UI just renders.
  | {
      type: "incomingCall";
      data: {
        callId: string;
        from: string;
        displayName: string;
        kind: "audio" | "video";
        expiresAtMs: number;
      };
    }
  | {
      // Wave 15 W15.6 — backend-emitted on start_dm_call after CallState
      // insert + CallInvite send. Frontends read full payload; no
      // per-frontend seed math.
      type: "callStarted";
      data: {
        callId: string;
        kind: "audio" | "video";
        peerKey: string;
        peerDisplayName: string;
        expiresAtMs: number;
      };
    }
  | {
      // Wave 14 W14.2 — payload extended with the data every frontend
      // needs to react identically. `expectedLocalCamera` is the
      // backend's "video calls expect camera-on" policy delivered as
      // data; Tauri starts WebCodecs capture; CLI/TUI ignore.
      type: "callConnected";
      data: {
        callId: string;
        kind: "audio" | "video";
        peerKey: string;
        peerDisplayName: string;
        expectedLocalCamera: boolean;
      };
    }
  | { type: "callTimedOut"; data: { callId: string } }
  | { type: "callMissed"; data: { callId: string; from: string } }
  | { type: "callDeclined"; data: { callId: string; reason: string } }
  | { type: "callEnded"; data: { callId: string; reason: string } }
  | {
      // Wave 13 — alerting hint: receiver got our CallInvite and is
      // ringing the user. Drives "Calling…" → "Ringing…" transition
      // on the OutgoingCallPanel.
      type: "callRinging";
      data: { callId: string };
    }
  | {
      // Wave 14 W14.3 — backend asks frontends to focus a conversation.
      // Emitted on call entry from both caller and receiver paths.
      // Tauri opens/focuses ChatWindow; CLI switches active prompt
      // context; TUI navigates.
      type: "conversationFocusRequested";
      data: {
        peerKey: string;
        displayName: string;
        reason: string;
      };
    }
  | {
      // Wave 12 W12.6 — peer flipped a media flag mid-call. Frontend
      // mounts/unmounts the corresponding tile.
      type: "callMediaStateChanged";
      data: {
        callId: string;
        audio: boolean;
        video: boolean;
        screen: boolean;
        timestampMs: number;
      };
    }
  | {
      // Wave 12 W12.11 — peer fired an emoji reaction during the call.
      type: "callReactionReceived";
      data: {
        callId: string;
        sender: string;
        emoji: string;
        timestampMs: number;
      };
    }
  | {
      // Wave 12 W12.9 — group call lifecycle events.
      type: "incomingGroupCall";
      data: {
        callId: string;
        from: string;
        displayName: string;
        kind: "audio" | "video";
        participants: string[];
        expiresAtMs: number;
      };
    }
  | { type: "groupCallConnected"; data: { callId: string } }
  | {
      type: "groupCallParticipantJoined";
      data: { callId: string; participantPubkey: string };
    }
  | {
      type: "groupCallParticipantLeft";
      data: { callId: string; participantPubkey: string; reason: string };
    }
  | {
      type: "groupCallEnded";
      data: { callId: string; reason: string };
    };
