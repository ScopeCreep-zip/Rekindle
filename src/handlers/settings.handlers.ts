import { commands } from "../ipc/commands";
import type { NetworkStatus } from "../ipc/commands";
import { setSettingsState } from "../stores/settings.store";
import type { SettingsState } from "../stores/settings.store";

export async function handleLoadSettings(): Promise<void> {
  try {
    const prefs = await commands.getPreferences();
    setSettingsState({
      notifications: prefs.notificationsEnabled,
      soundEnabled: prefs.notificationSound,
      autoStart: prefs.autoStart,
      startMinimized: prefs.startMinimized,
      showGameActivity: prefs.gameDetectionEnabled,
      autoAwayMinutes: prefs.autoAwayMinutes,
    });
  } catch (e) {
    console.error("Failed to load settings:", e);
  }
}

export async function handleSaveSettings(
  settings: Partial<SettingsState>,
): Promise<void> {
  try {
    const currentPrefs = await commands.getPreferences();
    const updated = {
      ...currentPrefs,
      ...(settings.notifications !== undefined && {
        notificationsEnabled: settings.notifications,
      }),
      ...(settings.soundEnabled !== undefined && {
        notificationSound: settings.soundEnabled,
      }),
      ...(settings.autoStart !== undefined && {
        autoStart: settings.autoStart,
      }),
      ...(settings.startMinimized !== undefined && {
        startMinimized: settings.startMinimized,
      }),
      ...(settings.showGameActivity !== undefined && {
        gameDetectionEnabled: settings.showGameActivity,
      }),
      ...(settings.autoAwayMinutes !== undefined && {
        autoAwayMinutes: settings.autoAwayMinutes,
      }),
    };
    await commands.setPreferences(updated);
    setSettingsState(settings);
  } catch (e) {
    console.error("Failed to save settings:", e);
  }
}

export async function handleSetAvatar(avatarData: number[]): Promise<void> {
  try {
    await commands.setAvatar(avatarData);
  } catch (e) {
    console.error("Failed to set avatar:", e);
  }
}

export async function handleCheckForUpdates(): Promise<boolean> {
  try {
    const available = await commands.checkForUpdates();
    return available;
  } catch (e) {
    console.error("Failed to check for updates:", e);
    return false;
  }
}

export async function handleGetNetworkStatus(): Promise<NetworkStatus | null> {
  try {
    return await commands.getNetworkStatus();
  } catch (e) {
    console.error("Failed to get network status:", e);
    return null;
  }
}

export async function handleGetGameStatus(): Promise<{
  gameId: number;
  gameName: string;
  elapsedSeconds: number;
} | null> {
  try {
    return await commands.getGameStatus();
  } catch (e) {
    console.error("Failed to get game status:", e);
    return null;
  }
}
