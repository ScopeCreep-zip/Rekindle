import { Channel } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "../invoke";
import type { ChatEvent, GroupDmEvent } from "./chat_events";
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
  OwnRecordsPhase,
  OwnRouteClass,
  RoutePauseCause,
  NotificationEvent,
  NotificationSubscriptionEvent,
} from "./notification_events";
import type { IntentReview, LifecycleState, SkinApplied, SkinTrial } from "../commands/dto";
import type { ActivityListItem, SettingsTab } from "../commands/types";
import type { InfoviewView } from "../commands/dto_infoview";

/// Where this window is in the host's event stream
/// (`rekindle_types::subscription_events::Cursor`, plan D3.2).
interface Cursor {
  epoch: number;
  seq: number;
}

/// What the backend sends on a window's event channel
/// (`src-tauri/src/event_router.rs::OutboundEnvelope`).
interface OutboundEnvelope {
  /// The host's cursor for a journaled event, else null.
  cursor: Cursor | null;
  /// The logical channel the payload belongs to (`chat-event`, …), or
  /// `event-stream` for the stream's own frames.
  channel: string;
  payload: unknown;
}

/// The stream's own frames (`event_router::STREAM_CHANNEL`).
const STREAM_CHANNEL = "event-stream";

type Handler = (payload: never) => void;

/// Handlers by logical channel. Every `subscribe*` function registers
/// here; the window's one backend stream feeds it.
const handlers = new Map<string, Set<Handler>>();

/// Offer `payload` to every handler of `channel`. A handler that throws is
/// reported and the rest still run. The throw must not escape: it would
/// unwind through the Tauri `Channel`'s callback before that advances its
/// next index (`@tauri-apps/api` 2.12.1 core.js:137-140), and every later
/// message to this window would wait forever behind it (walk rows M45 and
/// B61: the window stopped applying events after one reducer threw).
function dispatch(channel: string, payload: unknown): void {
  for (const handler of handlers.get(channel) ?? []) {
    try {
      (handler as (p: unknown) => void)(payload);
    } catch (err) {
      console.error(`a ${channel} handler failed`, err, payload);
    }
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

/// `sessionStorage` key for the last cursor this webview processed.
/// `sessionStorage` survives a reload of this webview only, so a reload
/// replays exactly what this window missed; a new window starts empty and
/// hydrates from the database instead.
const CURSOR_KEY = "rekindle.eventCursor";

function readCursor(): Cursor | null {
  try {
    const raw = sessionStorage.getItem(CURSOR_KEY);
    if (raw === null) return null;
    const parsed: unknown = JSON.parse(raw);
    if (
      typeof parsed === "object" && parsed !== null &&
      Number.isSafeInteger((parsed as Cursor).epoch) &&
      Number.isSafeInteger((parsed as Cursor).seq)
    ) {
      return parsed as Cursor;
    }
    return null;
  } catch {
    return null;
  }
}

function writeCursor(cursor: Cursor | null): void {
  try {
    if (cursor === null) {
      sessionStorage.removeItem(CURSOR_KEY);
    } else {
      sessionStorage.setItem(CURSOR_KEY, JSON.stringify(cursor));
    }
  } catch {
    // Storage unavailable — a reload then just doesn't replay.
  }
}

let stream: Promise<void> | null = null;

/// Resolves once this window's stream is registered with the backend: from
/// then on every event reaches the handlers, so a snapshot read after it
/// misses none (an event published between an earlier snapshot read and
/// the registration would be lost, plan VI1).
let markLive: () => void = () => {};
const live = new Promise<void>((resolve) => {
  markLive = resolve;
});

/// Wait until this window's event stream is live ([`startEventStream`]
/// resolved). A store that reads a snapshot reads it after this, so no
/// change between the snapshot and the subscription is lost.
export function eventStreamLive(): Promise<void> {
  return live;
}

/// Open this window's event stream. Call it once the window has
/// registered its handlers (end of its `onMount` subscriptions): a
/// reloading window is sent what it missed immediately. `reload` reloads
/// the window's state from the backend; it runs when the host cannot serve
/// the window's cursor (a new login or logout, or the window fell behind).
/// Idempotent per webview.
export function startEventStream(reload?: () => void): Promise<void> {
  if (stream) return stream;
  const from = readCursor();
  let last = from;
  const channel = new Channel<OutboundEnvelope>();
  channel.onmessage = (envelope) => {
    if (envelope.channel === STREAM_CHANNEL) {
      if (envelope.cursor === null) {
        // invalidSession: what this window holds may be stale.
        last = null;
        writeCursor(null);
        reload?.();
      } else {
        last = envelope.cursor;
        writeCursor(last);
      }
      return;
    }
    const cursor = envelope.cursor;
    if (cursor !== null) {
      // An event already seen (replayed and also delivered live around
      // the moment of subscription) is processed once.
      if (last !== null && cursor.epoch === last.epoch && cursor.seq <= last.seq) return;
      last = cursor;
      writeCursor(last);
    }
    dispatch(envelope.channel, envelope.payload);
  };
  stream = invoke<void>("subscribe_events", { channel, from }).then(markLive, (err) => {
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
 * Subscribe to group-DM lifecycle events (plan E2.6). They ride the
 * `chat-event` channel as `{ groupDm: … }`; unwrapped here so callers see
 * just the group event.
 */
export function subscribeGroupDmEvents(
  onEvent: (event: GroupDmEvent) => void,
): Promise<UnlistenFn> {
  return on<ChatEvent>("chat-event", (payload) => {
    if ("groupDm" in payload) onEvent(payload.groupDm);
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

/**
 * Subscribe to skin changes (`SystemEvent::SkinChanged`, plan E7.7.2):
 * every window, the login window included, receives them on `skin-event`.
 */
export function subscribeSkinEvents(onSkin: (skin: SkinApplied) => void): Promise<UnlistenFn> {
  return on<{ system: { skinChanged: { skin: SkinApplied } } }>("skin-event", (payload) => {
    onSkin(payload.system.skinChanged.skin);
  });
}

/**
 * Subscribe to "Keep this skin?" (`SystemEvent::SkinTrialChanged`, plan
 * E7.7.2): the Settings window receives the trial in progress, or `null`
 * once it is kept, reverted or replaced, on `skin-trial-event`.
 */
export function subscribeSkinTrial(
  onTrial: (trial: SkinTrial | null) => void,
): Promise<UnlistenFn> {
  return on<{ system: { skinTrialChanged: { trial: SkinTrial | null } } }>(
    "skin-trial-event",
    (payload) => onTrial(payload.system.skinTrialChanged.trial),
  );
}

/** The quick wipe's state the Host published when an arm or disarm committed. */
/**
 * Subscribe to new activity feed entries (`SystemEvent::ActivityAppended`,
 * plan E7.15): the buddy list and chat windows receive them on
 * `activity-event`, newest first, names resolved by the Host.
 */
export function subscribeActivity(
  onAppended: (entries: ActivityListItem[]) => void,
): Promise<UnlistenFn> {
  return on<{ system: { activityAppended: { entries: ActivityListItem[] } } }>(
    "activity-event",
    (payload) => onAppended(payload.system.activityAppended.entries),
  );
}

/**
 * Subscribe to this window's Infoview pushes (`SystemEvent::Infoview`,
 * plan E7.15 IV4): `infoview-event`, delivered to the asking window only.
 */
export function subscribeInfoview(
  onUpdated: (view: InfoviewView) => void,
): Promise<UnlistenFn> {
  return on<{ system: { infoview: { updated: { window: string; view: InfoviewView } } } }>(
    "infoview-event",
    (payload) => onUpdated(payload.system.infoview.updated.view),
  );
}

export interface QuickWipeChange {
  armed: boolean;
  shortcut: string | null;
}

/**
 * Subscribe to the quick wipe's state (`SystemEvent::QuickWipeChanged`,
 * plan E7.1): Settings receives it on `quick-wipe-event` once the Host
 * commits an arm or disarm; the shell registers the panic shortcut on the
 * same event.
 */
export function subscribeQuickWipe(
  onChange: (change: QuickWipeChange) => void,
): Promise<UnlistenFn> {
  return on<{ system: { quickWipeChanged: QuickWipeChange } }>("quick-wipe-event", (payload) =>
    onChange(payload.system.quickWipeChanged),
  );
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
 * The network indicator's bars (`NetworkEvent::Strength`), already
 * smoothed by the backend: the window draws what arrives (plan E7.2).
 */
export function subscribeNetworkStrength(onBars: (bars: number) => void): Promise<UnlistenFn> {
  return on<NetworkSubscriptionEvent>("network-status", (payload) => {
    const inner = payload.network;
    if ("strength" in inner) {
      onBars(inner.strength.bars);
    }
  });
}

/**
 * Whether our inbound route is paused (`NetworkEvent::RoutePaused`, plan
 * RV1c): the new-route budget is spent or unreadable, so the route stays down
 * until the Host's `RouteResumed` (the callback's null). Never a silent redraw.
 */
export function subscribeRoutePaused(onPause: (cause: RoutePauseCause | null) => void): Promise<UnlistenFn> {
  return on<NetworkSubscriptionEvent>("network-status", (payload) => {
    const inner = payload.network;
    if ("routePaused" in inner) onPause(inner.routePaused.cause);
    else if ("routeResumed" in inner) onPause(null);
  });
}

/** Own-route deaths above their baseline (`NetworkEvent::RouteDeathsAboveBaseline`, plan RV1): a warning. */
export function subscribeRouteDeaths(
  onDeaths: (d: { class: OwnRouteClass; deaths: number; young: number; windowSecs: number; baseline: number }) => void,
): Promise<UnlistenFn> {
  return on<NetworkSubscriptionEvent>("network-status", (payload) => {
    const inner = payload.network;
    if ("routeDeathsAboveBaseline" in inner) onDeaths(inner.routeDeathsAboveBaseline);
  });
}

/**
 * The Host's own-records stage (`NetworkEvent::OwnRecordsChanged`, plan
 * E2.5 FC3): whether our identity is published yet, and why not if it
 * failed.
 */
export function subscribeOwnRecords(
  onStage: (phase: OwnRecordsPhase, error: string | null) => void,
): Promise<UnlistenFn> {
  return on<NetworkSubscriptionEvent>("network-status", (payload) => {
    const inner = payload.network;
    if ("ownRecordsChanged" in inner) {
      onStage(inner.ownRecordsChanged.phase, inner.ownRecordsChanged.error);
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

/// Our own profile (name, avatar) changed: the Host's
/// `PresenceEvent::SelfProfileChanged` on the presence channel.
export function subscribeProfileUpdates(
  onUpdate: () => void,
): Promise<UnlistenFn> {
  return on<PresenceSubscriptionEvent>("presence-event", (payload) => {
    if ("selfProfileChanged" in payload.presence) onUpdate();
  });
}

/// A review of an action the backend began (`SubscriptionEvent::IntentPending`).
export function subscribeIntentEvents(
  onReview: (review: IntentReview) => void,
): Promise<UnlistenFn> {
  return on<{ intentPending: IntentReview }>("intent-event", (payload) => {
    onReview(payload.intentPending);
  });
}

/// A pairing link arrived from the OS and was refused
/// (`WebviewEvent::PairingLinkRefused`): pairing codes are entered in Settings.
export function subscribePairingLinkRefused(onRefused: () => void): Promise<UnlistenFn> {
  return on<null>("pairing-link-refused", () => {
    onRefused();
  });
}

/// The open settings window was asked to show a tab.
export function subscribeSettingsTab(onTab: (tab: SettingsTab) => void): Promise<UnlistenFn> {
  return on<SettingsTab>("settings-switch-tab", onTab);
}
