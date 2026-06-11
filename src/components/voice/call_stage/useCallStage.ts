import { createMemo, createSignal } from "solid-js";
import { voiceState } from "../../../stores/voice.store";
import { authState } from "../../../stores/auth.store";
import { communityState } from "../../../stores/community.store";
import { activePipeline } from "./pipeline_store";

/// Community voice keys `voiceState.participants` by the per-community
/// pseudonym (not the owner key), so "us" in that roster is the local
/// member's pseudonym for the community that owns the active voice
/// channel — found by matching `voiceState.channelId` to a community's
/// channel list (channel ids are globally unique). `null` until the
/// community's pseudonym is hydrated.
function selfPseudonymForActiveCall(): string | null {
  const chId = voiceState.channelId;
  if (!chId) return null;
  for (const c of Object.values(communityState.communities)) {
    if (c.channels.some((ch) => ch.id === chId)) {
      return c.myPseudonymKey;
    }
  }
  return null;
}

/// Durable display-name lookup for a call tile. The voice-participant
/// entry flaps with packet timeouts (a >5 s inbound voice stall removes
/// it and re-adds it on the next packet), so a tile named from it
/// degrades to the "Participant" placeholder mid-call whenever the
/// transport hiccups. The community member roster doesn't churn with
/// the transport — resolve from it first; the voice entry stays as the
/// fallback for members whose roster row hasn't hydrated yet.
function memberNameForActiveCall(pseudonym: string): string | null {
  const chId = voiceState.channelId;
  if (!chId) return null;
  for (const c of Object.values(communityState.communities)) {
    if (c.channels.some((ch) => ch.id === chId)) {
      const member = c.members.find((m) => m.pseudonymKey === pseudonym);
      return member && member.displayName !== "" ? member.displayName : null;
    }
  }
  return null;
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
  displayName: string;
  avatarUrl?: string;
  publicKey?: string;
  isLocal: boolean;
  /// Screen-share tile — preferred for the spotlight slot.
  isScreen: boolean;
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

    if (voiceState.isConnected) {
      const camOn = pipe?.cameraOn() ?? false;
      const scrOn = pipe?.screenOn() ?? false;
      if (pipe && camOn) {
        put(out, "self-camera", {
          bindVideo: pipe.bindCameraVideo,
          displayName: `${authState.displayName ?? "You"} (you)`,
          isLocal: true,
          isScreen: false,
        });
      }
      if (pipe && scrOn) {
        put(out, "self-screen", {
          bindVideo: pipe.bindScreenVideo,
          displayName: "Your screen",
          isLocal: true,
          isScreen: true,
        });
      }
      if (!camOn && !scrOn) {
        put(out, "self-avatar", {
          displayName: `${authState.displayName ?? "You"} (you)`,
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
      if (p) withVideo.add(p.publicKey);
      put(out, `remote-${r.streamId}`, {
        canvas: r.canvas,
        displayName:
          memberNameForActiveCall(r.senderPseudonym) ?? p?.displayName ?? "Participant",
        publicKey: p?.publicKey ?? r.senderPseudonym,
        isLocal: false,
        isScreen: false,
      });
    }

    // Audio-only participants → avatar tiles. Skip ourselves — the local
    // user is already rendered by the self-* tiles above, and the join
    // flow adds us to `voiceState.participants` too (keyed by our
    // community pseudonym), so without this guard we'd show a second
    // (avatar) card for the local user.
    const selfKey = selfPseudonymForActiveCall();
    for (const p of voiceState.participants) {
      if (selfKey && p.publicKey === selfKey) continue;
      if (withVideo.has(p.publicKey)) continue;
      put(out, `participant-${p.publicKey}`, {
        // Roster name first: the voice entry's displayName is the raw
        // pseudonym hex when the join-time member_names snapshot missed
        // this member.
        displayName: memberNameForActiveCall(p.publicKey) ?? p.displayName,
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
