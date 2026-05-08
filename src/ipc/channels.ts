import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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

export type PresenceEvent =
  | { type: "friendOnline"; data: { publicKey: string } }
  | { type: "friendOffline"; data: { publicKey: string } }
  | {
      type: "statusChanged";
      data: {
        publicKey: string;
        status: string;
        statusMessage: string | null;
      };
    }
  | {
      type: "gameChanged";
      data: {
        publicKey: string;
        gameName: string | null;
        gameId: number | null;
        elapsedSeconds: number | null;
        serverAddress: string | null;
      };
    };

export type VoiceEvent =
  | {
      // Emitted by backend after local node successfully joins a voice channel.
      // Carries activeCallType so the frontend doesn't decide that locally —
      // fixes C1 (VideoCallPanel never mounted because activeCallType was null).
      type: "localJoined";
      data: { channelId: string; activeCallType: "community" | "dm" };
    }
  | {
      type: "userJoined";
      data: { publicKey: string; displayName: string };
    }
  | { type: "userLeft"; data: { publicKey: string } }
  | {
      type: "userSpeaking";
      data: { publicKey: string; speaking: boolean };
    }
  | {
      type: "userMuted";
      data: { publicKey: string; muted: boolean };
    }
  | { type: "connectionQuality"; data: { quality: string } }
  | {
      type: "deviceChanged";
      data: { deviceType: string; deviceName: string; reason: string };
    }
  | {
      // Wave 14 W14.4 — backend tells us audio packets were dropped
      // since the last 1s tick. Frontend may surface as a toast or
      // status-bar indicator so the user sees "audio interrupted"
      // instead of confused silence.
      type: "packetsDropped";
      data: { reason: string; count: number };
    };

export type CommunityEvent =
  | {
      type: "joinAccepted";
      data: { communityId: string };
    }
  | {
      // Architecture §18.4 — eager-fetched expression bytes have landed.
      // Frontend should re-pull list_expressions for this community so
      // the picker re-renders with the resolved inline_data_base64.
      type: "expressionAssetReady";
      data: { communityId: string; expressionId: string };
    }
  | {
      type: "memberJoined";
      data: {
        communityId: string;
        pseudonymKey: string;
        displayName: string;
        roleIds: number[];
      };
    }
  | {
      type: "memberRemoved";
      data: { communityId: string; pseudonymKey: string };
    }
  | {
      type: "raidDetected";
      data: {
        communityId: string;
        joinsInWindow: number;
        maxJoinsPerInterval: number;
        joinIntervalSeconds: number;
      };
    }
  | {
      // Architecture §28.8 — sender pre-fetched OpenGraph metadata.
      type: "linkPreviewReceived";
      data: {
        communityId: string;
        senderPseudonym: string;
        channelId: string;
        messageId: string;
        url: string;
        title?: string;
        description?: string;
        imageUrl?: string;
        siteName?: string;
        fetchedAt: number;
      };
    }
  | {
      // Architecture §10.6 — reassembled MEK-decrypted video frame.
      type: "videoFrame";
      data: {
        communityId: string;
        senderPseudonym: string;
        streamId: string;
        frameSeq: number;
        keyframe: boolean;
        timestamp: number;
        payloadB64: string;
      };
    }
  | {
      // Architecture §10.6 — receiver bandwidth ack drives encoder bitrate.
      type: "videoFrameAck";
      data: {
        communityId: string;
        senderPseudonym: string;
        channelId: string;
        streamId: string;
        lastFrameSeq: number;
        kbps: number;
        lossQ8: number;
      };
    }
  | {
      // Architecture §10.6 — receiver requests an I-frame.
      type: "videoKeyframeRequest";
      data: {
        communityId: string;
        senderPseudonym: string;
        channelId: string;
        streamId: string;
      };
    }
  | {
      // Architecture §10.6 — out-of-band bandwidth advertisement.
      type: "videoBandwidthEstimate";
      data: {
        communityId: string;
        senderPseudonym: string;
        channelId: string;
        kbps: number;
        windowSecs: number;
        lossQ8: number;
      };
    }
  | {
      // Architecture §10.6 — peer's decode capabilities for adaptive sender.
      type: "videoMediaCapabilities";
      data: {
        communityId: string;
        senderPseudonym: string;
        channelId: string;
        maxPixelCount: number;
        maxFps: number;
        codecs: string[];
      };
    }
  | {
      // Architecture §10.6 + §22 — relay change (full mesh ↔ SFU).
      // `lamport` is the sender's per-community Lamport clock at the
      // moment of the topology decision; the reassembler uses it to
      // resolve simultaneous topology changes from multiple peers
      // (highest lamport wins, with sender_pseudonym as tiebreaker).
      type: "videoTopologyChange";
      data: {
        communityId: string;
        senderPseudonym: string;
        channelId: string;
        streamId: string;
        relayHostPseudonym: string | null;
        reason: string;
        lamport: number;
      };
    }
  | {
      type: "mekRotated";
      data: { communityId: string; channelId?: string; newGeneration: number };
    }
  | {
      type: "kicked";
      data: { communityId: string };
    }
  | {
      type: "rolesChanged";
      data: {
        communityId: string;
        roles: { id: number; name: string; color: number; permissions: string; position: number; hoist: boolean; mentionable: boolean; selfAssignable?: boolean }[];
      };
    }
  | {
      type: "memberRolesChanged";
      data: { communityId: string; pseudonymKey: string; roleIds: number[] };
    }
  | {
      type: "memberTimedOut";
      data: { communityId: string; pseudonymKey: string; timeoutUntil: number | null };
    }
  | {
      type: "channelOverwriteChanged";
      data: { communityId: string; channelId: string };
    }
  | {
      type: "governanceUpdated";
      data: { communityId: string };
    }
  | {
      type: "messageEdited";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
        newBody: string;
        editedAt: number;
      };
    }
  | {
      type: "messageDeleted";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
      };
    }
  | {
      type: "reactionAdded";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
        emoji: string;
        reactorPseudonym: string;
      };
    }
  | {
      type: "reactionRemoved";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
        emoji: string;
        reactorPseudonym: string;
      };
    }
  | {
      type: "messagePinned";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
        pinnedBy: string;
      };
    }
  | {
      type: "messageUnpinned";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
      };
    }
  | {
      type: "channelMessageDelivered";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
      };
    }
  | {
      type: "channelMessageDeliveryFailed";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
      };
    }
  | {
      type: "channelTyping";
      data: {
        communityId: string;
        channelId: string;
        pseudonymKey: string;
      };
    }
  | {
      type: "memberPresenceChanged";
      data: {
        communityId: string;
        pseudonymKey: string;
        status: string;
        gameName?: string;
        gameId?: number;
        elapsedSeconds?: number;
        serverAddress?: string;
      };
    }
  | {
      type: "autoModAlert";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
        ruleName: string;
      };
    }
  | {
      type: "threadCreated";
      data: {
        communityId: string;
        thread: {
          id: string;
          channelId: string;
          name: string;
          starterMessageId: string;
          creatorPseudonym: string;
          forumTag?: string | null;
          createdAt: number;
          archived: boolean;
          autoArchiveSeconds: number;
          lastMessageAt: number;
          messageCount: number;
        };
      };
    }
  | {
      type: "threadMessageReceived";
      data: {
        communityId: string;
        threadId: string;
        messageId: string;
        senderPseudonym: string;
        body: string;
        timestamp: number;
        replyToId: string | null;
      };
    }
  | {
      type: "threadArchived";
      data: {
        communityId: string;
        threadId: string;
        archived: boolean;
      };
    }
  | {
      type: "eventCreated";
      data: {
        communityId: string;
        event: {
          id: string;
          title: string;
          description: string;
          creatorPseudonym: string;
          startTime: number;
          endTime: number | null;
          channelId: string | null;
          maxAttendees: number | null;
          createdAt: number;
          status: string;
          rsvps: { pseudonymKey: string; status: string }[];
        };
      };
    }
  | {
      type: "eventUpdated";
      data: {
        communityId: string;
        event: {
          id: string;
          title: string;
          description: string;
          creatorPseudonym: string;
          startTime: number;
          endTime: number | null;
          channelId: string | null;
          maxAttendees: number | null;
          createdAt: number;
          status: string;
          rsvps: { pseudonymKey: string; status: string }[];
        };
      };
    }
  | {
      type: "eventDeleted";
      data: {
        communityId: string;
        eventId: string;
      };
    }
  | {
      type: "eventRsvpChanged";
      data: {
        communityId: string;
        eventId: string;
        pseudonymKey: string;
        status: string;
      };
    }
  | {
      type: "gameServerAdded";
      data: {
        communityId: string;
        server: {
          id: string;
          gameId: string;
          label: string;
          address: string;
          addedBy: string;
          createdAt: number;
        };
      };
    }
  | {
      type: "gameServerRemoved";
      data: {
        communityId: string;
        serverId: string;
      };
    }
  | {
      type: "eventReminder";
      data: {
        communityId: string;
        eventId: string;
        title: string;
        minutesUntilStart: number;
      };
    }
  | {
      type: "stageUpdate";
      data: {
        communityId: string;
        channelId: string;
        topic: string | null;
        speakers: string[];
        moderatorPseudonym: string;
      };
    }
  | {
      type: "speakRequest";
      data: {
        communityId: string;
        channelId: string;
        requesterPseudonym: string;
      };
    }
  | {
      type: "speakResponse";
      data: {
        communityId: string;
        channelId: string;
        requesterPseudonym: string;
        granted: boolean;
        moderatorPseudonym: string;
      };
    }
  | {
      type: "channelsUpdated";
      data: {
        communityId: string;
        channels: { id: string; name: string; channelType: string; categoryId?: string; topic?: string; slowmodeSeconds?: number }[];
        categories: { id: string; name: string; sortOrder: number }[];
      };
    }
  | {
      type: "inviteCreated";
      data: {
        communityId: string;
        codeHash: string;
        createdBy: string;
        maxUses: number | null;
        uses: number;
        expiresAt: number | null;
        createdAt: number;
      };
    }
  | {
      type: "inviteRevoked";
      data: { communityId: string; codeHash: string };
    }
  | {
      type: "inviteUsed";
      data: { communityId: string; codeHash: string; newUseCount: number };
    }
  | {
      type: "membersRefreshed";
      data: { communityId: string };
    }
  | {
      type: "systemMessage";
      data: {
        communityId: string;
        body: string;
        timestamp: number;
      };
    }
  | {
      type: "raidAlert";
      data: {
        communityId: string;
        active: boolean;
      };
    }
  | {
      type: "channelLockdown";
      data: {
        communityId: string;
        locked: boolean;
      };
    }
  | {
      type: "onboardingComplete";
      data: {
        communityId: string;
        pseudonymKey: string;
        roleIds: number[];
      };
    }
  | {
      type: "joinRejected";
      data: {
        communityId: string;
        reason: string;
      };
    }
  | {
      type: "syncComplete";
      data: {
        communityId: string;
        channelId: string;
        messageCount: number;
      };
    }
  | {
      type: "communityUpdated";
      data: {
        communityId: string;
        name: string | null;
        description: string | null;
        iconHash: string | null;
        bannerHash: string | null;
      };
    }
  | {
      type: "attachmentDownloaded";
      data: {
        communityId: string;
        channelId: string;
        attachmentId: string;
        localPath: string;
      };
    }
  | {
      type: "memberDiscovered";
      data: {
        communityId: string;
        pseudonymKey: string;
        displayName: string;
        subkeyIndex: number;
      };
    }
  | {
      type: "voiceJoin";
      data: {
        communityId: string;
        channelId: string;
        pseudonymKey: string;
        routeBlob: number[];
      };
    }
  | {
      type: "voiceLeave";
      data: {
        communityId: string;
        channelId: string;
        pseudonymKey: string;
      };
    }
  | {
      type: "voiceModeSwitch";
      data: {
        communityId: string;
        channelId: string;
        mode: string;
        hostPseudonym: string | null;
      };
    }
  | {
      // Architecture §10.9 — peer triggered a soundboard sound in a
      // voice channel. Frontend looks up the cached expression by
      // `expressionId` and plays the audio at `soundMeta.volume`.
      type: "soundboardPlay";
      data: {
        communityId: string;
        channelId: string;
        expressionId: string;
        actorPseudonym: string;
      };
    };

export type NotificationEvent =
  | {
      type: "messageReceived";
      data: {
        title: string;
        body: string;
        communityId: string;
        channelId: string;
        /**
         * Resolved per-channel/per-community sound override
         * (architecture §32 Phase 7 Week 25). `null` means the
         * frontend should fall back to its bundled default sound.
         */
        soundRef: string | null;
      };
    }
  | { type: "systemAlert"; data: { title: string; body: string } }
  | { type: "updateAvailable"; data: { version: string } }
  | {
      // P3.3 — peer requested a Signal session reset. Frontend MUST show
      // a confirmation modal displaying the safety_number for OOB
      // verification before invoking commands.acceptSessionReset.
      type: "sessionResetRequested";
      data: {
        peerPublicKey: string;
        peerDisplayName: string;
        safetyNumber: string;
      };
    }
  | {
      // Wave 12 W12.3 — sibling to chat-event::incomingCall, used as the
      // OS-level / CLI-frontend ring channel. Carries call_id so the
      // ringtone start/stop is correlatable to the call's lifecycle.
      type: "callIncoming";
      data: {
        callId: string;
        from: string;
        displayName: string;
        kind: "audio" | "video";
        expiresAtMs: number;
        isGroup: boolean;
      };
    };

export type NetworkStatusEvent = {
  attachmentState: string;
  isAttached: boolean;
  publicInternetReady: boolean;
  hasRoute: boolean;
};

/**
 * Safe listen wrapper — no-ops in E2E mode where Tauri event system
 * is unavailable (running in a regular browser, not a Tauri webview).
 */
function safeListen<T>(
  event: string,
  handler: (event: { payload: T }) => void,
): Promise<UnlistenFn> {
  if (import.meta.env.VITE_E2E === "true") {
    return Promise.resolve(() => {});
  }
  return listen<T>(event, handler);
}

export function subscribeChatEvents(
  onEvent: (event: ChatEvent) => void,
): Promise<UnlistenFn> {
  return safeListen<ChatEvent>("chat-event", (event) => {
    onEvent(event.payload);
  });
}

export function subscribePresenceEvents(
  onEvent: (event: PresenceEvent) => void,
): Promise<UnlistenFn> {
  return safeListen<PresenceEvent>("presence-event", (event) => {
    onEvent(event.payload);
  });
}

export function subscribeVoiceEvents(
  onEvent: (event: VoiceEvent) => void,
): Promise<UnlistenFn> {
  return safeListen<VoiceEvent>("voice-event", (event) => {
    onEvent(event.payload);
  });
}

export function subscribeCommunityEvents(
  onEvent: (event: CommunityEvent) => void,
): Promise<UnlistenFn> {
  return safeListen<CommunityEvent>("community-event", (event) => {
    onEvent(event.payload);
  });
}

export function subscribeNotificationEvents(
  onEvent: (event: NotificationEvent) => void,
): Promise<UnlistenFn> {
  return safeListen<NotificationEvent>("notification-event", (event) => {
    onEvent(event.payload);
  });
}

export function subscribeNetworkStatus(
  onEvent: (event: NetworkStatusEvent) => void,
): Promise<UnlistenFn> {
  return safeListen<NetworkStatusEvent>("network-status", (event) => {
    onEvent(event.payload);
  });
}

export function subscribeProfileUpdates(
  onUpdate: () => void,
): Promise<UnlistenFn> {
  return safeListen<null>("profile-updated", () => {
    onUpdate();
  });
}

export interface DeepLinkAction {
  action: string;
  communityId: string;
  inviteCode: string;
}

export function subscribeDeepLinkEvents(
  onEvent: (event: DeepLinkAction) => void,
): Promise<UnlistenFn> {
  return safeListen<DeepLinkAction>("deep-link-action", (event) => {
    onEvent(event.payload);
  });
}
