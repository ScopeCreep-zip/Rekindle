import type { CommunityEvent } from "../../ipc/channels";
import type { CommunitySubscriptionEvent } from "../../ipc/channels/community_subscription_events";
import type { VoiceSubscriptionEvent } from "../../ipc/channels/voice_events";
import { setCommunityState, communityState } from "../../stores/community.store";
import { commands } from "../../ipc/commands";
import { addToast } from "../../stores/toast.store";
import { announce } from "../../stores/announce.store";
import { settingsState } from "../../stores/settings.store";
import { voiceState, setVoiceState } from "../../stores/voice.store";
import { setVideoSessionConfig } from "../../stores/video.store";
import { refreshStageHandRaises } from "../../actions/community/shared";

/// Mirror signaling membership into the call-UI roster
/// (`voiceState.participants`), keyed by per-community pseudonym — the
/// same key `useCallStage` and the receive loop use. Only the channel
/// we're actively connected to drives this roster; self is rendered
/// separately so it's skipped. Decouples the roster from MEK-decrypt
/// (§10.1/§10.5): a member appears as soon as it's signaled, not only
/// once we can decrypt its audio.
function mirrorRosterAdd(
  communityId: string,
  channelId: string,
  entries: { pseudonymKey: string; displayName: string | null }[],
): void {
  if (voiceState.activeCallType !== "community" || voiceState.channelId !== channelId) return;
  const community = communityState.communities[communityId];
  const self = community?.myPseudonymKey;
  // Handshake-carried name wins (it never depends on registry-scan
  // timing); members store is the fallback; truncated key is last.
  const nameFor = (pk: string, carried: string | null): string =>
    carried ??
    community?.members.find((m) => m.pseudonymKey === pk)?.displayName ??
    pk.slice(0, 8);
  for (const { pseudonymKey: pk, displayName } of entries) {
    if (pk === self) continue;
    setVoiceState("participants", (prev) => {
      const existing = prev.findIndex((p) => p.publicKey === pk);
      if (existing >= 0) {
        // Upgrade a truncated-key placeholder with the carried name.
        if (displayName && prev[existing].displayName !== displayName) {
          return prev.map((p, i) => (i === existing ? { ...p, displayName } : p));
        }
        return prev;
      }
      return [
        ...prev,
        {
          publicKey: pk,
          displayName: nameFor(pk, displayName),
          isMuted: false,
          isSpeaking: false,
        },
      ];
    });
  }
}

/// Voice / stage / soundboard slice of the community event dispatcher.
/// Returns `true` when the event was consumed.
export function reduceVoice(event: CommunityEvent): boolean {
  if (event.type === "videoSessionConfig") {
    // Phase B / C — backend negotiated a new room-wide encoder +
    // decoder shape. Cache it; both the WebCodecs encoder
    // (video_sender.ts) and decoder (useVideoCallPanel.ts) read from
    // the store and reconfigure on change.
    const { communityId, channelId, config } = event.data;
    setVideoSessionConfig(communityId, channelId, config);
    return true;
  } else if (event.type === "videoCodecIncompatible") {
    // Phase 3 — no local encode codec is decodable by every peer in
    // the call. Backend latches the transition, so this fires once —
    // a single toast, no spam. Voice keeps working.
    const { peers } = event.data;
    const names = peers.map((p) => p.slice(0, 8)).join(", ");
    addToast(
      `Video unavailable: no compatible codec with ${names || "current peers"}`,
      "error",
    );
    return true;
  } else if (event.type === "videoEnvelopeRejected") {
    // Phase F — a gossiped video envelope failed signature or shape
    // verification at the receive boundary. Surface a warn-level toast
    // so the asymmetric-drop case is observable from the UI without
    // grepping structured logs.
    const { senderPseudonym, reason } = event.data;
    addToast(
      `Dropped video from ${senderPseudonym.slice(0, 8)}: ${reason}`,
      "error",
    );
    return true;
  } else if (event.type === "soundboardPlay") {
    // Architecture §10.9 — peer triggered a soundboard sound. The
    // backend already gated permissions, rate-limit, and cooldown;
    // we look up the cached expression and play it locally.
    const { communityId, channelId, expressionId } = event.data;
    if (!settingsState.soundEnabled) return true;
    if (voiceState.isDeafened) return true;
    const community = communityState.communities[communityId];
    if (!community) return true;
    const expression = (community.expressions ?? []).find((e) => e.id === expressionId);
    const dataUrl = expression?.inlineDataUrl;
    if (!dataUrl) return true;
    const _ = channelId;
    try {
      const audio = new Audio(dataUrl);
      const exprVolume = expression?.soundMeta?.volume;
      const expr = typeof exprVolume === "number" ? Math.min(Math.max(exprVolume, 0), 1) : 1.0;
      const out = Math.min(Math.max(voiceState.outputVolume, 0), 1);
      audio.volume = expr * out;
      void audio.play().catch((e) => {
        console.warn("soundboard playback failed:", e);
      });
    } catch (e) {
      console.warn("soundboard playback failed:", e);
    }
    return true;
  }
  return false;
}

/// MEK rotation on the daemon vocabulary.
export function reduceSubscriptionCrypto(event: CommunitySubscriptionEvent): void {
  if (!("crypto" in event)) return;
  const c = event.crypto;
  if (!("mekRotated" in c)) return;

  const { community, channel, generation } = c.mekRotated;
  if (!communityState.communities[community]) return;

  if (channel === null) {
    setCommunityState("communities", community, "mekGeneration", generation);
    return;
  }

  const idx = communityState.communities[community].channels.findIndex(
    (ch) => ch.id === channel,
  );
  if (idx >= 0) {
    setCommunityState("communities", community, "channels", idx, "mekGeneration", generation);
  }
  // Architecture §7.2 + §10.7 — voice MEK rotates on every join/leave
  // for forward+backward secrecy. Surface a cue when it is the channel
  // the user is actively connected to, so they know keys advanced
  // (e.g. a new speaker just joined the stage).
  if (voiceState.activeCallType === "community" && voiceState.channelId === channel) {
    announce("Voice keys rotated", "polite");
  }
}

// ── Daemon vocabulary ──────────────────────────────────────────────
//
// Each body is the `{ type, data }` case it replaced. Only the
// destructuring changed: `communityId`/`channelId` come out of the
// scope, and the roster's participants carry `displayName: string |
// null` where the old DTO had `string | undefined`.

/// Voice signalling on the daemon vocabulary.
///
/// Arrives on `voice-event`, not `community-event`: `VoiceEvent` is one
/// family and the backend routes the whole of it there. `scope` tells a
/// community call from a DM call, and this reducer only handles the
/// former — a DM call has no community channel state to update.
export function reduceSubscriptionVoice(event: VoiceSubscriptionEvent): void {
  const v = event.voice;

  if ("joined" in v) {
    const { scope, pseudonym, displayName } = v.joined;
    if (!("community" in scope)) return;
    const { community, channel } = scope.community;
    setCommunityState("voiceChannels", channel, (prev) => {
      const state = prev ?? { participants: [], mode: "mesh" as const, hostPseudonym: null };
      if (state.participants.includes(pseudonym)) return state;
      return { ...state, participants: [...state.participants, pseudonym] };
    });
    mirrorRosterAdd(community, channel, [
      { pseudonymKey: pseudonym, displayName },
    ]);
    return;
  }

  if ("rosterUpdated" in v) {
    // §10.1/§10.5 — a present member's catch-up roster. Tells a joiner
    // about everyone already in the channel (including members that
    // joined before us, whose `joined` we never received).
    const { scope, participants } = v.rosterUpdated;
    if (!("community" in scope)) return;
    const { community, channel } = scope.community;
    setCommunityState("voiceChannels", channel, (prev) => {
      const state = prev ?? { participants: [], mode: "mesh" as const, hostPseudonym: null };
      const merged = Array.from(
        new Set([...state.participants, ...participants.map((p) => p.pseudonymKey)]),
      );
      return { ...state, participants: merged };
    });
    mirrorRosterAdd(
      community,
      channel,
      participants.map((p) => ({
        pseudonymKey: p.pseudonymKey,
        displayName: p.displayName,
      })),
    );
    return;
  }

  if ("joinHandshake" in v) {
    // Local three-way join progress: announced → seen → connected.
    const { scope, state } = v.joinHandshake;
    if (!("community" in scope)) return;
    const channel = scope.community.channel;
    if (voiceState.activeCallType !== "community" || voiceState.channelId !== channel) return;
    if (state === "seen" || state === "connected") {
      setVoiceState("joinHandshake", state);
      if (state === "connected") {
        announce("Voice channel connected", "polite");
      }
    }
    return;
  }

  if ("mediaReady" in v) {
    // Backend media-ready gate transition. The control bar disables
    // camera/screen-share until ready; the camera effect in
    // useVideoCallPanel re-fires when this flips true.
    const { scope, ready, reason } = v.mediaReady;
    if (!("community" in scope)) return;
    const channel = scope.community.channel;
    if (voiceState.activeCallType === "community" && voiceState.channelId === channel) {
      setVoiceState("mediaReady", { ready, reason });
    }
    return;
  }

  if ("peerConfirmed" in v) {
    // A joiner finished its handshake — render them solid.
    const { scope, pseudonym } = v.peerConfirmed;
    if (!("community" in scope)) return;
    const channel = scope.community.channel;
    if (voiceState.activeCallType === "community" && voiceState.channelId === channel) {
      setVoiceState("participants", (prev) =>
        prev.map((p) => (p.publicKey === pseudonym ? { ...p, isConfirmed: true } : p)),
      );
    }
    return;
  }

  if ("left" in v) {
    const { scope, pseudonym } = v.left;
    if (!("community" in scope)) return;
    const channel = scope.community.channel;
    setCommunityState("voiceChannels", channel, (prev) => {
      if (!prev) return prev;
      return { ...prev, participants: prev.participants.filter((p) => p !== pseudonym) };
    });
    if (voiceState.activeCallType === "community" && voiceState.channelId === channel) {
      setVoiceState("participants", (prev) => prev.filter((p) => p.publicKey !== pseudonym));
    }
    return;
  }

  if ("modeChanged" in v) {
    const { scope, mode, hostPseudonym } = v.modeChanged;
    if (!("community" in scope)) return;
    const channel = scope.community.channel;
    // Update voice channel state
    setCommunityState("voiceChannels", channel, (prev) => {
      const state = prev ?? { participants: [], mode: "mesh" as const, hostPseudonym: null };
      return { ...state, mode: mode as "mesh" | "mcu", hostPseudonym };
    });
    // Trigger the Rust set_voice_mode command so our local transport/MCU loop updates
    commands.setVoiceMode(mode, hostPseudonym ?? undefined).catch((e) => {
      console.error("Failed to set voice mode:", e);
    });
    return;
  }

  if ("stageUpdated" in v) {
    const { scope, topic, speakers, moderatorPseudonym } = v.stageUpdated;
    if (!("community" in scope)) return;
    const { community, channel } = scope.community;
    setCommunityState(
      "communities", community, "channels",
      (ch) => ch.id === channel,
      (ch) => ({
        ...ch,
        topic: topic ?? ch.topic,
        stageSpeakers: speakers,
        stageModerator: moderatorPseudonym,
      }),
    );
    setCommunityState("voiceChannels", channel, (prev) => {
      const state = prev ?? { participants: [], mode: "mcu" as const, hostPseudonym: null };
      return {
        ...state,
        mode: "mcu",
        speakers,
        moderatorPseudonym,
        topic: topic ?? state.topic ?? null,
      };
    });
    void refreshStageHandRaises(community, channel);
    return;
  }

  if ("speakRequested" in v) {
    const { scope, requesterPseudonym } = v.speakRequested;
    if (!("community" in scope)) return;
    const channel = scope.community.channel;
    setCommunityState("voiceChannels", channel, (prev) => {
      const state = prev ?? { participants: [], mode: "mcu" as const, hostPseudonym: null };
      const pendingRequests = state.pendingRequests ?? [];
      if (pendingRequests.includes(requesterPseudonym)) return state;
      return { ...state, pendingRequests: [...pendingRequests, requesterPseudonym] };
    });
    addToast(`Speak request from ${requesterPseudonym.slice(0, 12)}`, "info");
    return;
  }

  if ("speakResponded" in v) {
    const { scope, requesterPseudonym, granted } = v.speakResponded;
    if (!("community" in scope)) return;
    const { community, channel } = scope.community;
    setCommunityState("voiceChannels", channel, (prev) => {
      if (!prev) return prev;
      return {
        ...prev,
        pendingRequests: (prev.pendingRequests ?? []).filter(
          (value) => value !== requesterPseudonym,
        ),
      };
    });
    void refreshStageHandRaises(community, channel);
    addToast(
      granted ? "Request to speak approved" : "Request to speak denied",
      granted ? "success" : "info",
    );
  }
}
