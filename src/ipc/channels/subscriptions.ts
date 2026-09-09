import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ChatEvent } from "./chat_events";
import type {
  PresenceEvent,
  PresenceSubscriptionEvent,
} from "./presence_events";
import type {
  VoiceEvent,
  VoiceSubscriptionEvent,
} from "./voice_events";
import type { AnyCommunityEvent } from "./community_subscription_events";
import type {
  NetworkStatusEvent,
  NetworkSubscriptionEvent,
  NotificationEvent,
  NotificationSubscriptionEvent,
} from "./notification_events";
import type { LifecycleState } from "../commands/dto";

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

/**
 * Subscribe to presence changes.
 *
 * The backend emits the whole `SubscriptionEvent`, so the payload is
 * wrapped in a `presence` key — the same envelope every channel carries,
 * because a channel like `community-event` multiplexes several families.
 * Unwrapped here so callers see just the presence event.
 */
export function subscribePresenceEvents(
  onEvent: (event: PresenceEvent) => void,
): Promise<UnlistenFn> {
  return safeListen<PresenceSubscriptionEvent>("presence-event", (event) => {
    onEvent(event.payload.presence);
  });
}

export function subscribeVoiceEvents(
  onEvent: (event: VoiceEvent) => void,
): Promise<UnlistenFn> {
  return safeListen<VoiceSubscriptionEvent>("voice-event", (event) => {
    onEvent(event.payload.voice);
  });
}

/**
 * Subscribe to community events.
 *
 * The channel carries both the desktop's `{ type, data }` envelope and
 * the daemon vocabulary; `isLegacyCommunityEvent` discriminates them.
 */
export function subscribeCommunityEvents(
  onEvent: (event: AnyCommunityEvent) => void,
): Promise<UnlistenFn> {
  return safeListen<AnyCommunityEvent>("community-event", (event) => {
    onEvent(event.payload);
  });
}

export function subscribeNotificationEvents(
  onEvent: (event: NotificationEvent) => void,
): Promise<UnlistenFn> {
  return safeListen<NotificationSubscriptionEvent>("notification-event", (event) => {
    onEvent(event.payload.notification);
  });
}

/**
 * Subscribe to network attachment status.
 *
 * The channel now carries the whole `NetworkEvent` family, but the
 * indicator only cares about attachment, so the other variants (route
 * deaths, watch renewals) are filtered out here rather than pushed onto
 * every caller.
 */
export function subscribeNetworkStatus(
  onEvent: (event: NetworkStatusEvent) => void,
): Promise<UnlistenFn> {
  return safeListen<NetworkSubscriptionEvent>("network-status", (event) => {
    const inner = event.payload.network;
    if ("attachmentChanged" in inner) {
      onEvent(inner.attachmentChanged);
    }
  });
}

/**
 * Lifecycle FSM transitions (`setup.rs` emits `{ state, at_ms }`). The
 * derived lifecycle store observes this to surface readiness in the UI.
 * No-ops in E2E (no Tauri events) — the store is seeded via
 * `lifecycleCurrent()` instead.
 */
export function subscribeLifecycleEvents(
  onState: (state: LifecycleState) => void,
): Promise<UnlistenFn> {
  return safeListen<{ state: LifecycleState; at_ms: number }>(
    "lifecycle-event",
    (event) => {
      onState(event.payload.state);
    },
  );
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
  secretsRecordKey: string;
  inviteCode: string;
}

export function subscribeDeepLinkEvents(
  onEvent: (event: DeepLinkAction) => void,
): Promise<UnlistenFn> {
  return safeListen<DeepLinkAction>("deep-link-action", (event) => {
    onEvent(event.payload);
  });
}
