import { createStore } from "solid-js/store";

export interface SettingsState {
  notifications: boolean;
  soundEnabled: boolean;
  autoStart: boolean;
  startMinimized: boolean;
  showGameActivity: boolean;
}

const [settingsState, setSettingsState] = createStore<SettingsState>({
  notifications: true,
  soundEnabled: true,
  autoStart: false,
  startMinimized: true,
  showGameActivity: true,
});

export { settingsState, setSettingsState };
