import { Channel } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "../invoke";
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
import type { DeepLinkRequest, SettingsTab } from "../commands/types";

/// What the backend sends on a window's event channel
/// (`src-tauri/src/event_router.rs::OutboundEnvelope`).
interface OutboundEnvelope {
  /// Journal sequence number for journaled events, else null.
  seq: number | null;
  /// The logical channel the payload belongs to (`chat-event`, …).
  channel: string;
  payload: unknown;
}

type Handler = (payload: never) => void;

/// Handlers by logical channel. Every `subscribe*` function registers
/// here; the window's one backend stream feeds it.
const handlers = new Map<string, Set<Handler>>();

function dispatch(channel: string, payload: unknown): void {
  for (const handler of handlers.get(channel) ?? []) {
    (handler as (p: unknown) => void)(payload);
  }
}

/// Register `handler` for one logical channel of this window's stream.
function on<T>(channel: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  let set = handlers.get(channel);
  if (!set) {
    set = new Set();
    handlers.set(channel, set);
  }
  set.add(handler as Handler);
  return Promise.resolve(() => {
    set.delete(handler as Handler);
  });
}

/// `sessionStorage` key for the last journal sequence number this webview
/// processed. `sessionStorage` survives a reload of this webview only, so
/// a reload replays exactly what this window missed; a new window starts
/// empty and hydrates from the database instead.
const SEQ_KEY = "rekindle.eventSeq";

function readSeq(): number | null {
  try {
    const raw = sessionStorage.getItem(SEQ_KEY);
    if (raw === null) return null;
    const n = Number(raw);
    return Number.isSafeInteger(n) && n >= 0 ? n : null;
  } catch {
    return null;
  }
}

function writeSeq(seq: number): void {
  try {
    sessionStorage.setItem(SEQ_KEY, String(seq));
  } catch {
    // Storage unavailable — a reload then just doesn't replay.
  }
}

let stream: Promise<void> | null = null;

/// Open this window's event stream. Call it once the window has
/// registered its handlers (end of its `onMount` subscriptions): a
/// reloading window is sent its missed events immediately. Idempotent per
/// webview; a no-op under E2E, which has no Tauri event transport.
export function startEventStream(): Promise<void> {
  if (import.meta.env.VITE_E2E === "true") return Promise.resolve();
  if (stream) return stream;
  const lastSeq = readSeq();
  let maxSeq = lastSeq ?? 0;
  const channel = new Channel<OutboundEnvelope>();
  channel.onmessage = (envelope) => {
    if (envelope.seq !== null) {
      // A journaled event already seen (replayed and also delivered live
      // around the moment of subscription) is processed once.
      if (envelope.seq <= maxSeq) return;
      maxSeq = envelope.seq;
      writeSeq(maxSeq);
    }
    dispatch(envelope.channel, envelope.payload);
  };
  stream = invoke<void>("subscribe_events", { channel, lastSeq }).catch((err) => {
    stream = null;
    console.error("subscribe_events failed — this window receives no events", err);
  });
  return stream;
}

export function subscribeChatEvents(
  onEvent: (event: ChatEvent) => void,
): Promise<UnlistenFn> {
  return on<ChatEvent>("chat-event", (payload) => {
    onEvent(payload);
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
  return on<PresenceSubscriptionEvent>("presence-event", (payload) => {
    onEvent(payload.presence);
  });
}

export function subscribeVoiceEvents(
  onEvent: (event: VoiceEvent) => void,
): Promise<UnlistenFn> {
  return on<VoiceSubscriptionEvent>("voice-event", (payload) => {
    onEvent(payload.voice);
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
  return on<AnyCommunityEvent>("community-event", (payload) => {
    onEvent(payload);
  });
}

export function subscribeNotificationEvents(
  onEvent: (event: NotificationEvent) => void,
): Promise<UnlistenFn> {
  return on<NotificationSubscriptionEvent>("notification-event", (payload) => {
    onEvent(payload.notification);
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
  return on<NetworkSubscriptionEvent>("network-status", (payload) => {
    const inner = payload.network;
    if ("attachmentChanged" in inner) {
      onEvent(inner.attachmentChanged);
    }
  });
}

/**
 * Lifecycle FSM transitions (`{ state, at_ms }`). The derived lifecycle
 * store observes this to surface readiness in the UI. Under E2E there is
 * no event stream; the store is seeded via `lifecycleCurrent()` instead.
 */
export function subscribeLifecycleEvents(
  onState: (state: LifecycleState) => void,
): Promise<UnlistenFn> {
  return on<{ state: LifecycleState; at_ms: number }>(
    "lifecycle-event",
    (payload) => {
      onState(payload.state);
    },
  );
}

export function subscribeProfileUpdates(
  onUpdate: () => void,
): Promise<UnlistenFn> {
  return on<null>("profile-updated", () => {
    onUpdate();
  });
}

/// Fires when an OS deep link is held for consent. The payload is the
/// same `DeepLinkRequest` that `getPendingDeepLink` returns.
export function subscribeDeepLinkEvents(
  onEvent: (event: DeepLinkRequest) => void,
): Promise<UnlistenFn> {
  return on<DeepLinkRequest>("deep-link-action", (payload) => {
    onEvent(payload);
  });
}

/// The open settings window was asked to show a tab.
export function subscribeSettingsTab(onTab: (tab: SettingsTab) => void): Promise<UnlistenFn> {
  return on<SettingsTab>("settings-switch-tab", onTab);
}
