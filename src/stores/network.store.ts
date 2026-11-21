import { onCleanup, onMount } from "solid-js";
import { createStore } from "solid-js/store";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { handleGetNetworkStatus } from "../actions/settings.actions";
import { subscribeNetworkStatus, subscribeNetworkStrength, subscribeOwnRecords } from "../ipc/channels";
import type { OwnRecordsPhase, RouteAvailability } from "../ipc/channels/notification_events";

/**
 * The node's network state, as the Host reports it: `get_network_status`
 * once when the first tracker mounts, then only the `network-status` push
 * events (plan E7.2 deleted the 30 s poll). One store per window: the
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
   * request or a first message waits for.
   */
  ownRecords: OwnRecordsPhase;
  /** Why publishing failed, when it did. */
  ownRecordsError: string | null;
}

const [networkState, setNetworkState] = createStore<NetworkState>({
  attachmentState: "detached",
  isAttached: false,
  publicInternetReady: false,
  hasRoute: false,
  mediaRoute: "idle",
  canWrite: false,
  bars: 0,
  ownRecords: "pending",
  ownRecordsError: null,
});

export { networkState };

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

/** The state when the first tracker mounts; the push events follow it. */
async function readNetworkStatus(): Promise<void> {
  const status = await handleGetNetworkStatus();
  if (status) {
    setNetworkState({
      attachmentState: status.attachmentState,
      isAttached: status.isAttached,
      publicInternetReady: status.publicInternetReady,
      hasRoute: status.hasRoute,
      mediaRoute: status.mediaRoute,
      canWrite: status.canWrite,
      bars: status.bars,
      ownRecords: status.ownRecords,
      ownRecordsError: status.ownRecordsError,
    });
  }
}

function keep(subscription: Promise<UnlistenFn>): void {
  void subscription.then((fn) => {
    if (users > 0) unlisteners.push(fn);
    else fn();
  });
}

function start(): void {
  void readNetworkStatus();
  keep(
    subscribeNetworkStatus((event) => {
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
  keep(subscribeNetworkStrength((bars) => setNetworkState("bars", bars)));
  keep(
    subscribeOwnRecords((phase, error) =>
      setNetworkState({ ownRecords: phase, ownRecordsError: error }),
    ),
  );
}

function stop(): void {
  for (const fn of unlisteners) fn();
  unlisteners = [];
}

/**
 * Keep the store current while the calling component is mounted. Several
 * components may track at once; the first reads the state and
 * subscribes, the last to unmount stops them.
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
