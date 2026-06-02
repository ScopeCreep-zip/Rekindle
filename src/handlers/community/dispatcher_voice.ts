import type { CommunityEvent } from "../../ipc/channels";
import { setCommunityState, communityState } from "../../stores/community.store";
import { commands } from "../../ipc/commands";
import { addToast } from "../../stores/toast.store";
import { announce } from "../../components/common/AnnounceRegion";
import { settingsState } from "../../stores/settings.store";
import { voiceState } from "../../stores/voice.store";
import { refreshStageHandRaises } from "./shared";

/// Voice / stage / soundboard slice of the community event dispatcher.
/// Returns `true` when the event was consumed.
export function reduceVoice(event: CommunityEvent): boolean {
  if (event.type === "mekRotated") {
    const { communityId, channelId, newGeneration } = event.data;
    if (communityState.communities[communityId]) {
      if (channelId) {
        const idx = communityState.communities[communityId].channels.findIndex((channel) => channel.id === channelId);
        if (idx >= 0) {
          setCommunityState("communities", communityId, "channels", idx, "mekGeneration", newGeneration);
        }
        // Architecture §7.2 + §10.7 — voice MEK rotates on every
        // join/leave for forward+backward secrecy. Surface a toast
        // when it's the channel the user is actively connected to
        // so they have a visible cue that keys advanced (e.g., a
        // new speaker just joined the stage).
        if (voiceState.activeCallType === "community" && voiceState.channelId === channelId) {
          announce("Voice keys rotated", "polite");
        }
      } else {
        setCommunityState("communities", communityId, "mekGeneration", newGeneration);
      }
    }
    return true;
  } else if (event.type === "voiceJoin") {
    const { channelId, pseudonymKey } = event.data;
    setCommunityState("voiceChannels", channelId, (prev) => {
      const state = prev ?? { participants: [], mode: "mesh" as const, hostPseudonym: null };
      if (state.participants.includes(pseudonymKey)) return state;
      return { ...state, participants: [...state.participants, pseudonymKey] };
    });
    return true;
  } else if (event.type === "voiceLeave") {
    const { channelId, pseudonymKey } = event.data;
    setCommunityState("voiceChannels", channelId, (prev) => {
      if (!prev) return prev;
      return { ...prev, participants: prev.participants.filter((p) => p !== pseudonymKey) };
    });
    return true;
  } else if (event.type === "voiceModeSwitch") {
    const { channelId, mode, hostPseudonym } = event.data;
    // Update voice channel state
    setCommunityState("voiceChannels", channelId, (prev) => {
      const state = prev ?? { participants: [], mode: "mesh" as const, hostPseudonym: null };
      return { ...state, mode: mode as "mesh" | "mcu", hostPseudonym };
    });
    // Trigger the Rust set_voice_mode command so our local transport/MCU loop updates
    commands.setVoiceMode(mode, hostPseudonym ?? undefined).catch((e) => {
      console.error("Failed to set voice mode:", e);
    });
    return true;
  } else if (event.type === "stageUpdate") {
    const { communityId, channelId, topic, speakers, moderatorPseudonym } = event.data;
    setCommunityState("communities", communityId, "channels",
      (channel) => channel.id === channelId,
      (channel) => ({
        ...channel,
        topic: topic ?? channel.topic,
        stageSpeakers: speakers,
        stageModerator: moderatorPseudonym,
      }),
    );
    setCommunityState("voiceChannels", channelId, (prev) => {
      const state = prev ?? { participants: [], mode: "mcu" as const, hostPseudonym: null };
      return {
        ...state,
        mode: "mcu",
        speakers,
        moderatorPseudonym,
        topic: topic ?? state.topic ?? null,
      };
    });
    void refreshStageHandRaises(communityId, channelId);
    return true;
  } else if (event.type === "speakRequest") {
    const { channelId, requesterPseudonym } = event.data;
    setCommunityState("voiceChannels", channelId, (prev) => {
      const state = prev ?? { participants: [], mode: "mcu" as const, hostPseudonym: null };
      const pendingRequests = state.pendingRequests ?? [];
      if (pendingRequests.includes(requesterPseudonym)) return state;
      return { ...state, pendingRequests: [...pendingRequests, requesterPseudonym] };
    });
    addToast(`Speak request from ${requesterPseudonym.slice(0, 12)}`, "info");
    return true;
  } else if (event.type === "speakResponse") {
    const { communityId, channelId, requesterPseudonym, granted } = event.data;
    setCommunityState("voiceChannels", channelId, (prev) => {
      if (!prev) return prev;
      return {
        ...prev,
        pendingRequests: (prev.pendingRequests ?? []).filter((value) => value !== requesterPseudonym),
      };
    });
    void refreshStageHandRaises(communityId, channelId);
    addToast(granted ? "Request to speak approved" : "Request to speak denied", granted ? "success" : "info");
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
