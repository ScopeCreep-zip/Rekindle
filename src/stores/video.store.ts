import { createStore } from "solid-js/store";
import type { SessionVideoConfig } from "../ipc/commands";

/**
 * Phase B / C — per-(community, channel) `SessionVideoConfig` cache.
 *
 * The backend's `rekindle_video::policy::negotiate_session_config()`
 * runs every time room membership or a peer's `MediaCapabilities`
 * changes, and emits `CommunityEvent::VideoSessionConfig` whenever the
 * negotiated shape differs from the last emission. Both the WebCodecs
 * encoder (`video_sender.ts`) and decoder (`useVideoCallPanel.ts`)
 * configure from this store — there is no client-side codec choice,
 * no `optimizeForLatency` probe, no L1T2 branch.
 *
 * The key is `communityId + ":" + channelId` — a stable composite that
 * survives navigating between channels in the same community without
 * collision.
 *
 * Reactive note: a SolidJS store of `Record<string, …>` is shallow-
 * reactive on the keys but deeply reactive inside each value. Callers
 * use `videoSessionConfigFor(...)` in a reactive primitive (createMemo
 * / createEffect) — re-evaluating when either the keyed value is
 * inserted, replaced, or mutated.
 */

interface VideoStoreState {
  /** Negotiated session config, keyed by `${communityId}:${channelId}`. */
  configs: Record<string, SessionVideoConfig>;
}

const [videoState, setVideoState] = createStore<VideoStoreState>({
  configs: {},
});

function configKey(communityId: string, channelId: string): string {
  return `${communityId}:${channelId}`;
}

export function setVideoSessionConfig(
  communityId: string,
  channelId: string,
  config: SessionVideoConfig,
): void {
  setVideoState("configs", configKey(communityId, channelId), config);
}

/** Read the current negotiated config; `undefined` until the backend
 *  has emitted `VideoSessionConfig` for this room. Encoder and decoder
 *  code paths both gate on this — see `useVideoCallPanel.ts` and
 *  `video_sender.ts`. */
export function videoSessionConfigFor(
  communityId: string,
  channelId: string,
): SessionVideoConfig | undefined {
  return videoState.configs[configKey(communityId, channelId)];
}

export { videoState };
