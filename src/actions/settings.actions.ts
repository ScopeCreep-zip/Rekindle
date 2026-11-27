import { commands } from "../ipc/commands";
import type { NetworkStatus } from "../ipc/commands";
import type { Preferences } from "../ipc/commands/types";
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
      notificationContent: prefs.notificationContent ?? "nameOnly",
    });
  } catch (e) {
    console.error("Failed to load settings:", e);
  }
}

/** Each Settings field `handleSaveSettings` stores, and the preference it is stored as. */
const PREFERENCE_OF: readonly (readonly [keyof SettingsState, keyof Preferences])[] = [
  ["notifications", "notificationsEnabled"],
  ["soundEnabled", "notificationSound"],
  ["autoStart", "autoStart"],
  ["startMinimized", "startMinimized"],
  ["autoVolunteerRelayForNewFriends", "autoVolunteerRelayForNewFriends"],
  ["ringtoneEnabled", "ringtoneEnabled"],
  ["ringtoneVolume", "ringtoneVolume"],
  ["inCallDndAutoEnable", "inCallDndAutoEnable"],
  ["notificationContent", "notificationContent"],
];

export async function handleSaveSettings(
  settings: Partial<SettingsState>,
): Promise<void> {
  try {
    const updated: Preferences = { ...(await commands.getPreferences()) };
    // Each setting the caller gave replaces its stored preference.
    for (const [setting, pref] of PREFERENCE_OF) {
      const value = settings[setting];
      if (value !== undefined) Object.assign(updated, { [pref]: value });
    }
    await commands.setPreferences(updated);
    setSettingsState(settings);
  } catch (e) {
    console.error("Failed to save settings:", e);
  }
}

/// Auto-away's threshold (0 off): the Host stores it and applies it at once
/// (an auto-away whose reason went ends), answering what it stored.
export async function handleSetAutoAway(minutes: number): Promise<void> {
  try {
    const prefs = await commands.setAutoAway(minutes);
    setSettingsState({ autoAwayMinutes: prefs.autoAwayMinutes });
  } catch (e) {
    console.error("Failed to set auto-away:", e);
  }
}

/// Turn game detection on or off: the Host starts or stops its scanner at
/// once and answers with what it stored, which the switch shows.
export async function handleSetGameDetection(enabled: boolean): Promise<void> {
  try {
    const prefs = await commands.setGameDetection(enabled);
    setSettingsState({ showGameActivity: prefs.gameDetectionEnabled });
  } catch (e) {
    addToast(`Game detection not changed: ${String(e)}`, "error");
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
