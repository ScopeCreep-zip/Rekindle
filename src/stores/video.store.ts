import { createStore } from "solid-js/store";
import type { SessionVideoConfig } from "../ipc/commands";

/**
 * Phase B / C — per-(community, channel) `SessionVideoConfig` cache.
 *
 * The backend's `rekindle_video::policy::negotiate_session_config()`
 * runs every time room membership or a peer's `MediaCapabilities`
 * changes, and emits `CommunityEvent::VideoSessionConfig` whenever the
 * negotiated shape differs from the last emission. The WebCodecs
 * encoder (`video_sender.ts`) configures from this store in community
 * mode (the backend already made the per-node codec pick); decoders
 * follow per-frame codec tags. DM mode is the one client-side pick —
 * see `dmPeerDecodeCodecs` below.
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
  /**
   * Phase 5 — DM peer's advertised video decode codecs (wire strings,
   * preference-ordered), keyed by peer pubkey hex. Sourced from their
   * CallInvite/CallAccept via `dmPeerVideoDecodeCodecs`. The DM video
   * sender intersects its own encode set against this; an absent or
   * empty entry means "unknown — use the VP9 floor".
   */
  dmPeerDecodeCodecs: Record<string, string[]>;
}

const [videoState, setVideoState] = createStore<VideoStoreState>({
  configs: {},
  dmPeerDecodeCodecs: {},
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

export function setDmPeerDecodeCodecs(
  peerPubkey: string,
  codecs: string[],
): void {
  setVideoState("dmPeerDecodeCodecs", peerPubkey, codecs);
}

/** The DM peer's decode codec wire strings, or `undefined` before the
 *  `dmPeerVideoDecodeCodecs` fetch lands. Read reactively by the DM
 *  video sender's `encoderConstraints()`. */
export function dmPeerDecodeCodecsFor(
  peerPubkey: string,
): string[] | undefined {
  return videoState.dmPeerDecodeCodecs[peerPubkey];
}

export { videoState };
