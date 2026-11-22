import { onCleanup, onMount } from "solid-js";
import { createStore } from "solid-js/store";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { handleGetNetworkStatus } from "../actions/settings.actions";
import {
  eventStreamLive,
  subscribeNetworkStatus,
  subscribeNetworkStrength,
  subscribeOwnRecords,
  subscribeRouteDeaths,
  subscribeRoutePaused,
} from "../ipc/channels";
import { addToast } from "./toast.store";
import type { OwnRecordsPhase, OwnRouteClass, RouteAvailability, RoutePauseCause } from "../ipc/channels/notification_events";

/**
 * The node's network state, as the Host reports it: the `network-status`
 * push events, subscribed when the first tracker mounts, and one
 * `get_network_status` snapshot read once the window's stream is live, so
 * nothing published before the subscription is lost (plan VI1; E7.2 deleted
 * the 30 s poll). One store per window: the
 * buddy list's indicator reads it, and so does every action that needs
 * the network before it may start (plan E7.13: Create waits for
 * `canCreateCommunity`).
 */
export interface NetworkState {
  attachmentState: string;
  isAttached: boolean;
  publicInternetReady: boolean;
  hasRoute: boolean;
  mediaRoute: RouteAvailability;
  /** The Host accepts writes now (its lifecycle's `can_write`). */
  canWrite: boolean;
  /**
   * Veilid's attachment strength as indicator bars, 0–5, smoothed by the
   * backend (`NetworkEvent::Strength`: a fall shows only once it lasted
   * 5 s, plan E7.2).
   */
  bars: number;
  /**
   * Whether our identity (profile and mailbox) is published yet (plan E2.5
   * FC3, `NetworkEvent::OwnRecordsChanged`): what an invite, a friend
   * request or a first message waits for. `unknown` until the snapshot or
   * the first `OwnRecordsChanged` says (plan FC4): the window never shows
   * a wait the Host did not report.
   */
  ownRecords: OwnRecordsPhase | "unknown";
  /** Why publishing failed, when it did. */
  ownRecordsError: string | null;
  /**
   * Our inbound route is paused (plan RV1, `NetworkEvent::RoutePaused`): the
   * new-route budget is spent or unreadable, so the route stays down rather
   * than drawing again, until the Host says otherwise.
   */
  routePaused: boolean;
  /** Why, as the Host states it, while paused. */
  routePauseCause: RoutePauseCause | null;
}

const [networkState, setNetworkState] = createStore<NetworkState>({
  attachmentState: "detached",
  isAttached: false,
  publicInternetReady: false,
  hasRoute: false,
  mediaRoute: "idle",
  canWrite: false,
  bars: 0,
  ownRecords: "unknown",
  ownRecordsError: null,
  routePaused: false,
  routePauseCause: null,
});

export { networkState };

/** The paused route's status line (owner decision 65): the Host's cause, worded. */
export function routePausedText(cause: RoutePauseCause | null): string {
  return cause === "unreadable"
    ? "Route budget couldn't be read; inbound route paused"
    : "Inbound route paused after repeated failures: possible attack";
}

/** The deaths warning, from the Host's numbers (plan RV1): what died, how often, against the usual. */
function routeDeathsText(d: { class: OwnRouteClass; deaths: number; windowSecs: number; baseline: number }): string {
  const minutes = Math.max(1, Math.round(d.windowSecs / 60));
  const route = d.class === "media" ? "media route" : "inbound route";
  return `Our ${route} failed ${d.deaths} times in ${minutes} min (usually ${d.baseline}): possible attack.`;
}

/** Public internet ready and a route: the indicator's "Connected". */
export function isNetworkConnected(): boolean {
  return networkState.publicInternetReady && networkState.hasRoute;
}

/**
 * A community create can start (plan E7.13, audit 2026-11-18): the Host
 * takes writes and the public internet is ready both ways, which is when
 * Veilid's DHT is online (0.5.8 `storage_manager/mod.rs:1096-1104`); no
 * private route is needed. Unlike a message, a create cannot be held and
 * sent later: the community's first record is signed with a one-time key
 * destroyed right after (ADR 0011), so a held write would have no writer.
 */
export function canCreateCommunity(): boolean {
  return networkState.canWrite && networkState.publicInternetReady;
}

/** Why a create must wait, in plain words. */
export const CREATE_WAIT_REASON =
  "a new community's first record is signed with a one-time key that is destroyed right after, so it cannot be saved and sent later";

export function isNetworkConnecting(): boolean {
  const state = networkState.attachmentState;
  return (
    state === "attaching" ||
    state === "attached_weak" ||
    (networkState.isAttached && !networkState.publicInternetReady)
  );
}

export type NetworkLabel = "Connected" | "Connecting..." | "No Route" | "Disconnected";

export function networkLabel(): NetworkLabel {
  if (isNetworkConnected()) return "Connected";
  if (isNetworkConnecting()) return "Connecting...";
  if (networkState.publicInternetReady && !networkState.hasRoute) return "No Route";
  return "Disconnected";
}

let users = 0;
let unlisteners: UnlistenFn[] = [];

/**
 * How many push events of each kind this window applied. Each event carries
 * its kind's whole state, read when it was sent, and the stream delivers in
 * order, so an event that arrives while the snapshot is in flight is never
 * older than the last change: the snapshot then leaves that kind alone
 * rather than overwrite it with what may be older (plan VI1).
 */
const applied = { attachment: 0, strength: 0, ownRecords: 0, routePaused: 0 };

/** The state when the stream went live; the push events follow it. */
async function readNetworkStatus(): Promise<void> {
  const before = { ...applied };
  const status = await handleGetNetworkStatus();
  if (!status) return;
  if (applied.attachment === before.attachment) {
    setNetworkState({
      attachmentState: status.attachmentState,
      isAttached: status.isAttached,
      publicInternetReady: status.publicInternetReady,
      hasRoute: status.hasRoute,
      mediaRoute: status.mediaRoute,
      canWrite: status.canWrite,
    });
  }
  if (applied.strength === before.strength) setNetworkState("bars", status.bars);
  if (applied.ownRecords === before.ownRecords) {
    setNetworkState({ ownRecords: status.ownRecords, ownRecordsError: status.ownRecordsError });
  }
}

function keep(subscription: Promise<UnlistenFn>): void {
  void subscription.then((fn) => {
    if (users > 0) unlisteners.push(fn);
    else fn();
  });
}

/** Subscribe first, then read the snapshot once the stream is live. */
function start(): void {
  keep(
    subscribeNetworkStatus((event) => {
      applied.attachment += 1;
      setNetworkState({
        attachmentState: event.attachmentState,
        isAttached: event.isAttached,
        publicInternetReady: event.publicInternetReady,
        hasRoute: event.hasRoute,
        mediaRoute: event.mediaRoute,
        canWrite: event.canWrite,
      });
    }),
  );
  keep(
    subscribeNetworkStrength((bars) => {
      applied.strength += 1;
      setNetworkState("bars", bars);
    }),
  );
  keep(
    subscribeOwnRecords((phase, error) => {
      applied.ownRecords += 1;
      setNetworkState({ ownRecords: phase, ownRecordsError: error });
    }),
  );
  keep(
    subscribeRoutePaused((paused, cause) => {
      applied.routePaused += 1;
      setNetworkState({ routePaused: paused, routePauseCause: paused ? cause : null });
    }),
  );
  // Deaths above their baseline: a notice with the Host's numbers, no reaction.
  keep(subscribeRouteDeaths((d) => addToast(routeDeathsText(d), "error")));
  void eventStreamLive().then(readNetworkStatus);
}

function stop(): void {
  for (const fn of unlisteners) fn();
  unlisteners = [];
}

/**
 * Keep the store current while the calling component is mounted. Several
 * components may track at once; the first subscribes and reads the state
 * once the window's stream is live (the window calls `startEventStream`),
 * the last to unmount stops them.
 */
export function trackNetworkStatus(): void {
  let mounted = false;
  onMount(() => {
    mounted = true;
    users += 1;
    if (users === 1) start();
  });
  onCleanup(() => {
    if (!mounted) return;
    users -= 1;
    if (users === 0) stop();
  });
}
