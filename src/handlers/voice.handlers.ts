import { commands } from "../ipc/commands";
import { voiceState, setVoiceState } from "../stores/voice.store";

export async function handleJoinVoice(channelId: string): Promise<void> {
  try {
    await commands.joinVoiceChannel(channelId);
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
    setVoiceState({
      isConnected: false,
      channelId: null,
      participants: [],
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
