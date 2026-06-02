import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ChatEvent } from "./chat_events";
import type { PresenceEvent } from "./presence_events";
import type { VoiceEvent } from "./voice_events";
import type { CommunityEvent } from "./community_events";
import type { NotificationEvent, NetworkStatusEvent } from "./notification_events";
import type { LifecycleState } from "../../stores/lifecycle.store";

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
  inviteCode: string;
}

export function subscribeDeepLinkEvents(
  onEvent: (event: DeepLinkAction) => void,
): Promise<UnlistenFn> {
  return safeListen<DeepLinkAction>("deep-link-action", (event) => {
    onEvent(event.payload);
  });
}
