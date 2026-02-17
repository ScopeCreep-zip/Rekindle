import type { UnlistenFn } from "@tauri-apps/api/event";
import { commands } from "../ipc/commands";
import { subscribeVoiceEvents } from "../ipc/channels";
import { voiceState, setVoiceState } from "../stores/voice.store";
import { friendsState, setFriendsState } from "../stores/friends.store";

let voiceEventUnlisten: UnlistenFn | null = null;

/** Subscribe to voice events from the backend and update the store. */
export async function initVoiceEventListener(): Promise<UnlistenFn> {
  return subscribeVoiceEvents((event) => {
    switch (event.type) {
      case "userJoined":
        setVoiceState("participants", (prev) => [
          ...prev.filter((p) => p.publicKey !== event.data.publicKey),
          {
            publicKey: event.data.publicKey,
            displayName: event.data.displayName,
            isMuted: false,
            isSpeaking: false,
          },
        ]);
        break;
      case "userLeft":
        setVoiceState(
          "participants",
          (prev) => prev.filter((p) => p.publicKey !== event.data.publicKey),
        );
        break;
      case "userSpeaking":
        setVoiceState(
          "participants",
          (p) => p.publicKey === event.data.publicKey,
          "isSpeaking",
          event.data.speaking,
        );
        break;
      case "userMuted":
        setVoiceState(
          "participants",
          (p) => p.publicKey === event.data.publicKey,
          "isMuted",
          event.data.muted,
        );
        break;
      case "connectionQuality":
        setVoiceState("connectionQuality", event.data.quality);
        break;
      case "deviceChanged":
        setVoiceState("deviceChangeCount", (prev) => prev + 1);
        break;
    }
  });
}

export async function handleJoinVoice(channelId: string): Promise<void> {
  try {
    await commands.joinVoiceChannel(channelId);

    // Subscribe to voice events
    voiceEventUnlisten = await initVoiceEventListener();

    setVoiceState({
      isConnected: true,
      channelId,
    });
  } catch (e) {
    console.error("Failed to join voice:", e);
  }
}

export async function handleLeaveVoice(): Promise<void> {
  try {
    await commands.leaveVoice();

    // Unsubscribe from voice events
    if (voiceEventUnlisten) {
      voiceEventUnlisten();
      voiceEventUnlisten = null;
    }

    setVoiceState({
      isConnected: false,
      channelId: null,
      participants: [],
      connectionQuality: "good",
      activeCallType: null,
    });
  } catch (e) {
    console.error("Failed to leave voice:", e);
  }
}

export async function handleToggleMute(): Promise<void> {
  try {
    const newMuted = !voiceState.isMuted;
    await commands.setMute(newMuted);
    setVoiceState("isMuted", newMuted);
  } catch (e) {
    console.error("Failed to toggle mute:", e);
  }
}

export async function handleToggleDeafen(): Promise<void> {
  try {
    const newDeafened = !voiceState.isDeafened;
    await commands.setDeafen(newDeafened);
    setVoiceState("isDeafened", newDeafened);
  } catch (e) {
    console.error("Failed to toggle deafen:", e);
  }
}

export function subscribeBuddyListVoiceEvents(): Promise<UnlistenFn> {
  return subscribeVoiceEvents((event) => {
    switch (event.type) {
      case "userJoined": {
        if (friendsState.friends[event.data.publicKey]) {
          setFriendsState("friends", event.data.publicKey, "voiceChannel", "active");
        }
        break;
      }
      case "userLeft": {
        if (friendsState.friends[event.data.publicKey]) {
          setFriendsState("friends", event.data.publicKey, "voiceChannel", null);
        }
        break;
      }
    }
  });
}
