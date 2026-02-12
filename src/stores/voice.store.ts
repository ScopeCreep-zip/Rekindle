import { createStore } from "solid-js/store";

export interface VoiceParticipant {
  publicKey: string;
  displayName: string;
  isMuted: boolean;
  isSpeaking: boolean;
}

export interface VoiceState {
  isConnected: boolean;
  channelId: string | null;
  isMuted: boolean;
  isDeafened: boolean;
  participants: VoiceParticipant[];
}

const [voiceState, setVoiceState] = createStore<VoiceState>({
  isConnected: false,
  channelId: null,
  isMuted: false,
  isDeafened: false,
  participants: [],
});

export { voiceState, setVoiceState };
