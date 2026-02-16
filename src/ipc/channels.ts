import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ChatEvent =
  | {
      type: "messageReceived";
      data: {
        from: string;
        body: string;
        timestamp: number;
        conversationId: string;
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
      data: { publicKey: string; displayName: string };
    }
  | { type: "friendRequestRejected"; data: { from: string } }
  | { type: "friendRemoved"; data: { publicKey: string } }
  | {
      type: "channelHistoryLoaded";
      data: {
        channelId: string;
        messages: {
          id: number;
          senderId: string;
          body: string;
          timestamp: number;
          isOwn: boolean;
        }[];
      };
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
      };
    };

export type VoiceEvent =
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
    };

export type CommunityEvent =
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
      type: "mekRotated";
      data: { communityId: string; newGeneration: number };
    }
  | {
      type: "kicked";
      data: { communityId: string };
    }
  | {
      type: "rolesChanged";
      data: {
        communityId: string;
        roles: { id: number; name: string; color: number; permissions: number; position: number; hoist: boolean; mentionable: boolean }[];
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
    };

export type NotificationEvent =
  | { type: "systemAlert"; data: { title: string; body: string } }
  | { type: "updateAvailable"; data: { version: string } };

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
