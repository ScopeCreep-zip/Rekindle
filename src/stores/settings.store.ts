import { createStore } from "solid-js/store";
import type { AudioDeviceInfo } from "../ipc/commands";

export interface SettingsState {
  notifications: boolean;
  soundEnabled: boolean;
  autoStart: boolean;
  startMinimized: boolean;
  showGameActivity: boolean;
  autoAwayMinutes: number;
  /** W11.3 — when ON, accepting a new friend request also volunteers
   *  a Strand Relay route for that friend (architecture §13). OFF by
   *  default; explicit consent gate per the vulnerable-user threat
   *  model. Per-friend, never network-wide. */
  autoVolunteerRelayForNewFriends: boolean;
  /** Architecture §32 W26 + plan §Failure 1 — enumerated input devices
   *  for the Settings → Audio dropdowns. Hydrated by `listAudioDevices`. */
  inputDevices: AudioDeviceInfo[];
  outputDevices: AudioDeviceInfo[];
  /** Persisted device id selected by the user. `null` means "system
   *  default" — the backend's `resolve_device` falls back accordingly
   *  (cpal `default_input_device`/`default_output_device`). */
  selectedInputDevice: string | null;
  selectedOutputDevice: string | null;
  /** Plan §Failure 2 — `videoDeviceId` from `Preferences`. Persisted
   *  through the same path as audio. WebView enumerates camera devices
   *  client-side so the dropdown lives entirely in the frontend. */
  selectedVideoDeviceId: string | null;
  /** W12.2 — gates the synthesized ring on incoming calls and the
   *  ringback on outgoing calls. Independent from `soundEnabled`
   *  (which only covers message notification sounds) so the user can
   *  silence message dings while still hearing call rings, or vice
   *  versa. */
  ringtoneEnabled: boolean;
  /** Linear volume for ringtone / ringback / busy tone, [0, 1]. */
  ringtoneVolume: number;
  /** Suppress OS notifications + message-arrival sounds while a call
   *  is active so a noisy chat doesn't distract the participants.
   *  In-app modals still surface. */
  inCallDndAutoEnable: boolean;
}

const [settingsState, setSettingsState] = createStore<SettingsState>({
  notifications: true,
  soundEnabled: true,
  autoStart: false,
  startMinimized: true,
  showGameActivity: true,
  autoAwayMinutes: 10,
  autoVolunteerRelayForNewFriends: false,
  inputDevices: [],
  outputDevices: [],
  selectedInputDevice: null,
  selectedOutputDevice: null,
  selectedVideoDeviceId: null,
  ringtoneEnabled: true,
  ringtoneVolume: 0.4,
  inCallDndAutoEnable: true,
});

export { settingsState, setSettingsState };
