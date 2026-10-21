import { commands } from "../ipc/commands";
import type { NetworkStatus } from "../ipc/commands";
import { setSettingsState } from "../stores/settings.store";
import type { SettingsState } from "../stores/settings.store";
import { addToast } from "../stores/toast.store";
import { asImageKind } from "../ipc/commands/system";

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
      autoVolunteerRelayForNewFriends:
        prefs.autoVolunteerRelayForNewFriends ?? false,
      ringtoneEnabled: prefs.ringtoneEnabled ?? true,
      ringtoneVolume: prefs.ringtoneVolume ?? 0.4,
      inCallDndAutoEnable: prefs.inCallDndAutoEnable ?? true,
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
      ...(settings.autoVolunteerRelayForNewFriends !== undefined && {
        autoVolunteerRelayForNewFriends: settings.autoVolunteerRelayForNewFriends,
      }),
      ...(settings.ringtoneEnabled !== undefined && {
        ringtoneEnabled: settings.ringtoneEnabled,
      }),
      ...(settings.ringtoneVolume !== undefined && {
        ringtoneVolume: settings.ringtoneVolume,
      }),
      ...(settings.inCallDndAutoEnable !== undefined && {
        inCallDndAutoEnable: settings.inCallDndAutoEnable,
      }),
    };
    await commands.setPreferences(updated);
    setSettingsState(settings);
  } catch (e) {
    console.error("Failed to save settings:", e);
  }
}

/// Set our avatar from a picked file's bytes and MIME type. A format the
/// Host does not accept, or a refused upload, is shown; `true` on success.
export async function handleSetAvatar(avatarData: number[], mime: string): Promise<boolean> {
  const kind = asImageKind(mime);
  if (!kind) {
    addToast("That picture can't be used: choose a PNG, JPEG, or GIF.", "error");
    return false;
  }
  try {
    await commands.setAvatar(avatarData, kind);
    return true;
  } catch (e) {
    addToast(`Avatar not changed: ${String(e)}`, "error");
    return false;
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
  serverAddress: string | null;
} | null> {
  try {
    return await commands.getGameStatus();
  } catch (e) {
    console.error("Failed to get game status:", e);
    return null;
  }
}
