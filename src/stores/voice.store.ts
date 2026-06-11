import { createStore } from "solid-js/store";

export interface VoiceParticipant {
  publicKey: string;
  displayName: string;
  isMuted: boolean;
  isSpeaking: boolean;
  /** Three-way handshake: this peer sent VoiceJoinConfirmed (it is
   *  transport-ready). Undefined/false renders as "connecting". */
  isConfirmed?: boolean;
}

export interface VoiceState {
  isConnected: boolean;
  channelId: string | null;
  isMuted: boolean;
  isDeafened: boolean;
  participants: VoiceParticipant[];
  connectionQuality: string;
  /** Phase 5 — receive-side jitter drops in the last 5 s window. */
  rxOverflowDrops: number;
  rxLateDrops: number;
  /** Inbound media dropped for MEK reasons (rotation race signal). */
  rxMekDrops: number;
  /** Cumulative inbound voice-channel drops since login. */
  ingressDrops: number;
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
  /** Local three-way join handshake state for community voice
   *  (backend-emitted): "announced" (VoiceJoin sent, nobody seen us
   *  yet), "seen" (a member acked), "connected" (confirmed sent).
   *  Null outside community calls. */
  joinHandshake: "announced" | "seen" | "connected" | null;
  /** Backend media-ready gate: video may only start when `ready`.
   *  `reason` names the next blocker ("handshake-seen", "mek-missing",
   *  …) for the connecting state. Null until the first transition. */
  mediaReady: { ready: boolean; reason: string } | null;
}

const [voiceState, setVoiceState] = createStore<VoiceState>({
  isConnected: false,
  channelId: null,
  isMuted: false,
  isDeafened: false,
  participants: [],
  connectionQuality: "good",
  rxOverflowDrops: 0,
  rxLateDrops: 0,
  rxMekDrops: 0,
  ingressDrops: 0,
  activeCallType: null,
  inputDevice: null,
  outputDevice: null,
  inputVolume: 1.0,
  outputVolume: 1.0,
  deviceChangeCount: 0,
  cameraOn: false,
  screenShareOn: false,
  joinHandshake: null,
  mediaReady: null,
});

export { voiceState, setVoiceState };
