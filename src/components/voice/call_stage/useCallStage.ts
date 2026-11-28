import { createMemo, createSignal } from "solid-js";
import { authState } from "../../../stores/auth.store";
import { communityState } from "../../../stores/community.store";
import { voiceState } from "../../../stores/voice.store";
import { activePipeline } from "./pipeline_store";

/// Community voice keys `voiceState.participants` by the per-community
/// pseudonym (not the owner key), so "us" in that roster is the local
/// member's pseudonym for the community that owns the active voice
/// channel — found by matching `voiceState.channelId` to a community's
/// channel list (channel ids are globally unique). `null` until the
/// community's pseudonym is hydrated.
function communityForActiveCall() {
  const channel = voiceState.channelId;
  if (!channel) return undefined;
  return Object.values(communityState.communities).find((community) =>
    community.channels.some((ch) => ch.id === channel)
  );
}

/// One cell in the call gallery. `publicKey` (when present) lets the tile
/// read live speaking/muted state reactively from the voice store, so the
/// membership list below doesn't churn on every speaking event.
export interface CallTile {
  key: string;
  /// Remote decoder canvas to mount, when this participant is sending video.
  canvas?: HTMLCanvasElement;
  /// Local-preview binder, for the self camera / screen tiles.
  bindVideo?: (el: HTMLVideoElement | null) => void;
  /// What the tile's label reads ("Alice (you)", "Your screen").
  displayName: string;
  /// The person's name as the Host serves it, for the avatar's initials:
  /// never the label made from it.
  servedName: string;
  avatarUrl?: string;
  publicKey?: string;
  isLocal: boolean;
  /// Screen-share tile — preferred for the spotlight slot.
  isScreen: boolean;
  /// Self CAMERA surfaces mirror (Discord-style); screens and remote
  /// tiles never do. Only meaningful for canvas tiles — the <video>
  /// preview mirrors via its own class.
  mirror?: boolean;
}

export function useCallStage() {
  const [spotlightKey, setSpotlightKey] = createSignal<string | null>(null);

  // Cache tiles by key so unchanged tiles keep their object identity across
  // recomputes — <For> then reuses the DOM (and the appended remote canvas)
  // instead of tearing it down. Speaking/muted are intentionally NOT read
  // here; ParticipantTile reads them reactively from the store.
  const cache = new Map<string, CallTile>();
  const put = (out: CallTile[], key: string, data: Omit<CallTile, "key">): void => {
    let tile = cache.get(key);
    if (tile) {
      Object.assign(tile, data);
    } else {
      tile = { key, ...data };
      cache.set(key, tile);
    }
    out.push(tile);
  };

  const tiles = createMemo<CallTile[]>(() => {
    const pipe = activePipeline();
    const out: CallTile[] = [];
    const community = communityForActiveCall();
    const selfServed =
      voiceState.activeCallType === "community" ? community?.myDisplayName : authState.displayName;

    if (voiceState.isConnected && selfServed) {
      const camOn = pipe?.cameraOn() ?? false;
      const scrOn = pipe?.screenOn() ?? false;
      // Self view, fed by ONE capture per platform (never a 2nd camera
      // open, never a loopback):
      //   - webview (mac/Windows): direct getUserMedia MediaStream bound
      //     to a <video> via bindCameraVideo.
      //   - Linux native: the backend tees a JPEG preview branch we paint
      //     to a canvas (nativeSelfCanvas).
      // `nativeCapture()` is decided at mount and stable for the session,
      // so the path never flips mid-session. CRITICAL: the cached tile
      // objects are plain (mutated via Object.assign) and the keyed <For>
      // reuses a row by reference — so a tile must NEVER change content
      // TYPE (canvas↔video) under the same key, or the row never
      // re-renders (black forever). On native, while the canvas hasn't
      // been created yet we therefore show the avatar under a DIFFERENT
      // key ("self-avatar"); the key swap forces a fresh row that mounts
      // the canvas the moment it exists.
      const selfName = `${selfServed} (you)`;
      if (pipe && camOn) {
        if (pipe.nativeCapture?.()) {
          const nativeCanvas = pipe.nativeSelfCanvas?.() ?? null;
          if (nativeCanvas) {
            put(out, "self-camera", {
              canvas: nativeCanvas,
              displayName: selfName,
              servedName: selfServed,
              isLocal: true,
              isScreen: false,
              mirror: true,
            });
          } else {
            put(out, "self-avatar", {
              displayName: selfName,
              servedName: selfServed,
              avatarUrl: authState.avatarUrl ?? undefined,
              isLocal: true,
              isScreen: false,
            });
          }
        } else {
          put(out, "self-camera", {
            bindVideo: pipe.bindCameraVideo,
            displayName: selfName,
            servedName: selfServed,
            isLocal: true,
            isScreen: false,
          });
        }
      }
      if (pipe && scrOn) {
        put(out, "self-screen", {
          bindVideo: pipe.bindScreenVideo,
          displayName: "Your screen",
          servedName: selfServed,
          isLocal: true,
          isScreen: true,
        });
      }
      if (!camOn && !scrOn) {
        put(out, "self-avatar", {
          displayName: selfName,
          servedName: selfServed,
          avatarUrl: authState.avatarUrl ?? undefined,
          isLocal: true,
          isScreen: false,
        });
      }
    }

    // Remote video tiles — one per live stream (a sender can have both a
    // camera and a screen stream). Matched to a participant by pseudonym
    // for the display name + speaking/muted overlay.
    const remotes = pipe?.remotes() ?? [];
    const withVideo = new Set<string>();
    for (const r of remotes) {
      const p = voiceState.participants.find((x) => x.publicKey === r.senderPseudonym);
      if (!p) continue;
      withVideo.add(p.publicKey);
      put(out, `remote-${r.streamId}`, {
        canvas: r.canvas,
        displayName: p.displayName,
        servedName: p.displayName,
        publicKey: r.senderPseudonym,
        isLocal: false,
        isScreen: false,
      });
    }

    // Audio-only participants → avatar tiles. Skip ourselves — the local
    // user is already rendered by the self-* tiles above, and the join
    // flow adds us to `voiceState.participants` too (keyed by our
    // community pseudonym), so without this guard we'd show a second
    // (avatar) card for the local user.
    const selfKey = community?.myPseudonymKey;
    for (const p of voiceState.participants) {
      if (selfKey && p.publicKey === selfKey) continue;
      if (withVideo.has(p.publicKey)) continue;
      put(out, `participant-${p.publicKey}`, {
        displayName: p.displayName,
        servedName: p.displayName,
        publicKey: p.publicKey,
        isLocal: false,
        isScreen: false,
      });
    }

    // Evict tiles that no longer exist so the cache can't grow unbounded.
    const live = new Set(out.map((t) => t.key));
    for (const key of [...cache.keys()]) {
      if (!live.has(key)) cache.delete(key);
    }
    return out;
  });

  // Spotlight: an explicit click wins; otherwise auto-promote a local
  // screen-share (Discord/Zoom behaviour). Remote screen vs camera isn't
  // distinguishable on the wire, so remote promotion is click-only.
  const spotlight = createMemo<CallTile | null>(() => {
    const all = tiles();
    const manual = spotlightKey();
    if (manual) {
      const hit = all.find((t) => t.key === manual);
      if (hit) return hit;
    }
    return all.find((t) => t.isScreen && t.isLocal) ?? null;
  });

  const filmstrip = createMemo<CallTile[]>(() => {
    const spot = spotlight();
    if (!spot) return [];
    return tiles().filter((t) => t.key !== spot.key);
  });

  function toggleSpotlight(key: string): void {
    setSpotlightKey((cur) => (cur === key ? null : key));
  }

  return { tiles, spotlight, filmstrip, toggleSpotlight };
}
