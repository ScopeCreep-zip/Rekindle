import type { UnlistenFn } from "@tauri-apps/api/event";
import { commands } from "../ipc/commands";
import { subscribeVoiceEvents } from "../ipc/channels";
import { voiceState, setVoiceState } from "../stores/voice.store";
import { friendsState, setFriendsState } from "../stores/friends.store";
import { addToast } from "../stores/toast.store";

let voiceEventUnlisten: UnlistenFn | null = null;

/** Subscribe to voice events from the backend and update the store. */
export async function initVoiceEventListener(): Promise<UnlistenFn> {
  return subscribeVoiceEvents((event) => {
    switch (event.type) {
      case "localJoined":
        // Backend-authoritative join state — every frontend (Tauri GUI, CLI,
        // future TUI) mirrors the same activeCallType from this single event.
        // Fixes C1: VideoCallPanel's <Show> gate at CommunityWindow.tsx:731-744
        // requires activeCallType === "community" but the prior frontend-only
        // set ran AFTER the gate evaluated, so the panel never mounted.
        setVoiceState({
          isConnected: true,
          channelId: event.data.channelId,
          activeCallType: event.data.activeCallType,
        });
        break;
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
      case "packetsDropped": {
        // W14.4 — backend tells us audio packets were dropped over
        // the last 1 s. Toast so the user sees an objective signal
        // ("audio interrupted") rather than confused silence. Backend
        // already logged details at info!/warn!.
        const { count, reason } = event.data;
        addToast(
          `Voice packets dropped: ${count} (${reason})`,
          "error",
        );
        break;
      }
    }
  });
}

export async function handleJoinVoice(channelId: string, communityId?: string): Promise<void> {
  try {
    // Subscribe BEFORE the command fires so the LocalJoined event the backend
    // emits during start_session reaches us — late subscription would miss it.
    if (!voiceEventUnlisten) {
      voiceEventUnlisten = await initVoiceEventListener();
    }
    await commands.joinVoiceChannel(channelId, communityId);
    // Backend emits VoiceEvent::LocalJoined which the listener mirrors into
    // voiceState (isConnected + channelId + activeCallType). No manual set
    // here — that was the C1 bug: prior code set isConnected/channelId but
    // not activeCallType, so the <Show> gate at CommunityWindow.tsx:731-744
    // never opened and VideoCallPanel never mounted.
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

export async function handleRequestToSpeak(
  communityId: string,
  channelId: string,
): Promise<void> {
  try {
    await commands.requestToSpeak(communityId, channelId);
  } catch (e) {
    console.error("Failed to request to speak:", e);
  }
}

export async function handleRespondToSpeakRequest(
  communityId: string,
  channelId: string,
  requesterPseudonym: string,
  granted: boolean,
): Promise<void> {
  try {
    await commands.respondToSpeakRequest(
      communityId,
      channelId,
      requesterPseudonym,
      granted,
    );
  } catch (e) {
    console.error("Failed to respond to speak request:", e);
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
