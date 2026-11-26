// The Host's voice state and devices (plan E4.1 S5), for every window that
// shows a voice control or a device choice: the community window (its
// voice panel and call bar), the buddy list (a skin's Mute) and Settings.
//
// Written only from the Host: the `statusChanged` and `devicesChanged`
// events on `voice-event` (the latest status is journaled, so a window
// opening mid-call gets it on its stream), and one request each on mount
// while the store is empty, in case the journal's replay came first.

import type { UnlistenFn } from "@tauri-apps/api/event";
import { commands } from "../ipc/commands";
import { subscribeVoiceEvents } from "../ipc/channels";
import { setSettingsState, settingsState } from "../stores/settings.store";
import { setVoiceState, voiceState } from "../stores/voice.store";
import { addToast } from "../stores/toast.store";
import { errorMessage } from "../utils/error";

/** Mirror the Host's voice status and devices into the stores. Register
 *  before `startEventStream`. */
export function subscribeVoiceStatus(): Promise<UnlistenFn> {
  return subscribeVoiceEvents((event) => {
    if ("statusChanged" in event) {
      setVoiceState("status", event.statusChanged.status);
    } else if ("devicesChanged" in event) {
      setSettingsState("inputDevices", event.devicesChanged.devices.inputDevices);
      setSettingsState("outputDevices", event.devicesChanged.devices.outputDevices);
    }
  });
}

/** Ask the Host for its voice status and devices, each only while its
 *  store is still empty; a refusal is shown in the Host's words. */
export async function loadVoiceStatus(): Promise<void> {
  const status = voiceState.status === null
    ? commands.getVoiceStatus().then((s) => {
        if (voiceState.status === null) setVoiceState("status", s);
      })
    : Promise.resolve();
  const devices = settingsState.inputDevices.length === 0 && settingsState.outputDevices.length === 0
    ? commands.listAudioDevices().then((d) => {
        setSettingsState("inputDevices", d.inputDevices);
        setSettingsState("outputDevices", d.outputDevices);
      })
    : Promise.resolve();
  await Promise.all([status, devices]).catch((e) => addToast(errorMessage(e), "error"));
}
