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
  connectionQuality: string;
  activeCallType: "dm" | "community" | null;
  inputDevice: string | null;
  outputDevice: string | null;
  inputVolume: number;
  outputVolume: number;
  deviceChangeCount: number;
}

const [voiceState, setVoiceState] = createStore<VoiceState>({
  isConnected: false,
  channelId: null,
  isMuted: false,
  isDeafened: false,
  participants: [],
  connectionQuality: "good",
  activeCallType: null,
  inputDevice: null,
  outputDevice: null,
  inputVolume: 1.0,
  outputVolume: 1.0,
  deviceChangeCount: 0,
});

export { voiceState, setVoiceState };
