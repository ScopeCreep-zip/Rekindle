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
  /** Architecture §10.6 — desired camera state. VoicePanel writes via
   *  the toggle button; VideoCallPanel reacts and starts/stops the
   *  WebCodecs pipeline. Storing in the voice store (rather than as
   *  component-local state inside VideoCallPanel) keeps the controls
   *  visible in VoicePanel even when VideoCallPanel is unmounted. */
  cameraOn: boolean;
  /** Architecture §10.6 — desired screen-share state. */
  screenShareOn: boolean;
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
  cameraOn: false,
  screenShareOn: false,
});

export { voiceState, setVoiceState };
