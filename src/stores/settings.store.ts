import { createStore } from "solid-js/store";
import type { AudioDeviceInfo } from "../ipc/commands";

export interface SettingsState {
  notifications: boolean;
  soundEnabled: boolean;
  autoStart: boolean;
  startMinimized: boolean;
  showGameActivity: boolean;
  autoAwayMinutes: number;
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
}

const [settingsState, setSettingsState] = createStore<SettingsState>({
  notifications: true,
  soundEnabled: true,
  autoStart: false,
  startMinimized: true,
  showGameActivity: true,
  autoAwayMinutes: 10,
  inputDevices: [],
  outputDevices: [],
  selectedInputDevice: null,
  selectedOutputDevice: null,
  selectedVideoDeviceId: null,
});

export { settingsState, setSettingsState };
