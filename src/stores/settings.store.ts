import { createStore } from "solid-js/store";

export interface SettingsState {
  notifications: boolean;
  soundEnabled: boolean;
  autoStart: boolean;
  startMinimized: boolean;
  showGameActivity: boolean;
  autoAwayMinutes: number;
}

const [settingsState, setSettingsState] = createStore<SettingsState>({
  notifications: true,
  soundEnabled: true,
  autoStart: false,
  startMinimized: true,
  showGameActivity: true,
  autoAwayMinutes: 10,
});

export { settingsState, setSettingsState };
