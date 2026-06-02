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
