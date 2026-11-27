/**
 * Device-level notifications, on the `notification-event` channel.
 *
 * These mirror `rekindle_types::subscription_events::NotificationEvent`
 * — the same vocabulary the CLI subscribes to. They replaced a
 * Tauri-only `channels::NotificationEvent`, which meant a CLI could not
 * ring, alert, or prompt without reimplementing the protocol logic
 * behind each one. `callIncoming` in particular was documented as
 * existing *for* a CLI notifier, which a Tauri-only enum prevented.
 *
 * Externally tagged (`{ variantName: {...} }`) rather than
 * `{ type, data }`: the same values cross the daemon IPC as postcard,
 * which cannot decode a tagged enum.
 */
export type NotificationEvent =
  | {
      messageReceived: {
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
  | {
      systemAlert: {
        title: string;
        body: string;
        subject: { displayName: string; unnamedBody: string } | null;
      };
    }
  | { updateAvailable: { version: string } }
  | {
      /**
       * Wave 12 W12.3 — sibling to `chat-event::incomingCall`, used as
       * the OS-level / CLI ring channel. Carries `callId` so ringtone
       * start/stop is correlatable to the call's lifecycle.
       */
      callIncoming: {
        callId: string;
        from: string;
        displayName: string;
        kind: "audio" | "video";
        expiresAtMs: number;
        isGroup: boolean;
      };
    };

/** The full subscription event as emitted on the channel. */
export type NotificationSubscriptionEvent = { notification: NotificationEvent };

/**
 * Network attachment state, on the `network-status` channel.
 *
 * Was a standalone `NetworkStatusEvent`; it is now the
 * `attachmentChanged` variant of Tier 1's `NetworkEvent`, which gained
 * `attachmentState` and the general route's availability (`generalRoute`,
 * plan F9-A) to carry what this already had, and `mediaRoute` for the
 * media-class route voice needs (plan C7.9c).
 */
type NetworkEvent =
  | {
      attachmentChanged: {
        attachmentState: string;
        isAttached: boolean;
        publicInternetReady: boolean;
        /** Our inbound (General) route's availability (plan F9-A). */
        generalRoute: RouteAvailability;
        mediaRoute: RouteAvailability;
        /** The Host accepts writes now (its lifecycle's `can_write`). */
        canWrite: boolean;
      };
    }
  | { strength: { bars: number } }
  | { ownRecordsChanged: { phase: OwnRecordsPhase; error: string | null } }
  | { localRoutesDied: { count: number } }
  /** Our new-route budget is spent (or unreadable), so the inbound route stays down (plan RV1c). */
  | { routePaused: { cause: RoutePauseCause } }
  /** The pause is over: a refill brought the route back (plan RV1c). */
  | { routeResumed: Record<string, never> }
  /** Our own `class` route is down and the Host is retrying (plan F9-A): since
   *  `sinceMs` (epoch ms), with Veilid's `reason`. */
  | { ownRouteDown: { class: OwnRouteClass; sinceMs: number; reason: string } }
  /** Our own `class` route is up again (plan F9-A). */
  | { ownRouteUp: { class: OwnRouteClass } }
  /** Own-route deaths of `class` in `windowSecs` rose above `baseline`: a warning only (plan RV1). */
  | {
      routeDeathsAboveBaseline: {
        class: OwnRouteClass;
        deaths: number;
        /** Of `deaths`, routes that died under 60 s old. */
        young: number;
        windowSecs: number;
        baseline: number;
      };
    }
  | { remoteRoutesDied: { peerKeys: string[] } }
  | { valueChanged: { recordKey: string; changedSubkeys: number[] } };

export type NetworkSubscriptionEvent = { network: NetworkEvent };

/** Why the inbound route is paused (Rust `RoutePauseCause`, plan RV1c): the budget is
 *  spent, or it could not be read. */
export type RoutePauseCause = "exhausted" | "unreadable";

/** One of our own route classes, as a route event names it (Rust `OwnRouteClass`). */
export type OwnRouteClass = "general" | "media";

/**
 * Where our own-records stage is (Rust `OwnRecordsPhase`, plan E2.5 FC3):
 * an invite, a friend request or a first message waits for `held`.
 */
export type OwnRecordsPhase = "pending" | "held" | "failed";

/**
 * What one of our route classes is doing (Rust `RouteAvailability`).
 * Voice and video need the media route; the general route is never
 * substituted for it.
 */
export type RouteAvailability = "idle" | "allocating" | "available" | "unavailable" | "failed";

/** Shape the network indicator reads, whatever variant arrived. */
export interface NetworkStatusEvent {
  attachmentState: string;
  isAttached: boolean;
  publicInternetReady: boolean;
  /** Our inbound (General) route's availability (plan F9-A). */
  generalRoute: RouteAvailability;
  mediaRoute: RouteAvailability;
  canWrite: boolean;
}
