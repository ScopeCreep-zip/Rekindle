import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ChatEvent =
  | {
      type: "MessageReceived";
      data: {
        from: string;
        body: string;
        timestamp: number;
        conversationId: string;
      };
    }
  | { type: "TypingIndicator"; data: { from: string; typing: boolean } }
  | { type: "MessageAck"; data: { messageId: number } }
  | {
      type: "FriendRequest";
      data: { from: string; displayName: string; message: string };
    }
  | {
      type: "FriendRequestAccepted";
      data: { from: string; displayName: string };
    }
  | {
      type: "FriendAdded";
      data: { publicKey: string; displayName: string };
    }
  | { type: "FriendRequestRejected"; data: { from: string } };

export type PresenceEvent =
  | { type: "FriendOnline"; data: { publicKey: string } }
  | { type: "FriendOffline"; data: { publicKey: string } }
  | {
      type: "StatusChanged";
      data: {
        publicKey: string;
        status: string;
        statusMessage: string | null;
      };
    }
  | {
      type: "GameChanged";
      data: {
        publicKey: string;
        gameName: string | null;
        gameId: number | null;
        elapsedSeconds: number | null;
      };
    };

export type VoiceEvent =
  | {
      type: "UserJoined";
      data: { publicKey: string; displayName: string };
    }
  | { type: "UserLeft"; data: { publicKey: string } }
  | {
      type: "UserSpeaking";
      data: { publicKey: string; speaking: boolean };
    }
  | {
      type: "UserMuted";
      data: { publicKey: string; muted: boolean };
    }
  | { type: "ConnectionQuality"; data: { quality: string } };

export type NotificationEvent =
  | { type: "SystemAlert"; data: { title: string; body: string } }
  | { type: "UpdateAvailable"; data: { version: string } };

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
