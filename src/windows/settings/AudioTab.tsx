import { Component, onMount, For } from "solid-js";
import FormField from "../../components/common/FormField";
import { settingsState, setSettingsState } from "../../stores/settings.store";
import { commands } from "../../ipc/commands";

const AudioTab: Component = () => {
  // Plan §Failure 1 — load enumerated audio devices + the persisted
  // selection when the audio tab opens. The voice engine's `resolve_device`
  // (crates/rekindle-voice/src/device.rs:38) matches by `cpal::Device::name()`
  // and falls back to system default if the saved id no longer exists.
  onMount(() => {
    commands.listAudioDevices().then((d) => {
      setSettingsState("inputDevices", d.inputDevices);
      setSettingsState("outputDevices", d.outputDevices);
    }).catch((e) => {
      console.error("Failed to enumerate audio devices:", e);
    });
    commands.getPreferences().then((prefs) => {
      setSettingsState("selectedInputDevice", prefs.inputDevice);
      setSettingsState("selectedOutputDevice", prefs.outputDevice);
    }).catch((e) => {
      console.error("Failed to load device preferences:", e);
    });
  });

  async function persistAudioSelection(): Promise<void> {
    try {
      await commands.setAudioDevices(
        settingsState.selectedInputDevice,
        settingsState.selectedOutputDevice,
      );
    } catch (e) {
      console.error("Failed to persist audio device selection:", e);
    }
  }

  return (
    <>
      <div class="settings-section-title">Audio Devices</div>
      <FormField label="Input Device">
        <select
          class="form-select"
          value={settingsState.selectedInputDevice ?? ""}
          onChange={(e) => {
            const next = e.currentTarget.value || null;
            setSettingsState("selectedInputDevice", next);
            void persistAudioSelection();
          }}
        >
          <option value="">System Default</option>
          <For each={settingsState.inputDevices}>
            {(d) => (
              <option value={d.id}>
                {d.name}{d.isDefault ? " (default)" : ""}
              </option>
            )}
          </For>
        </select>
      </FormField>
      <FormField label="Output Device">
        <select
          class="form-select"
          value={settingsState.selectedOutputDevice ?? ""}
          onChange={(e) => {
            const next = e.currentTarget.value || null;
            setSettingsState("selectedOutputDevice", next);
            void persistAudioSelection();
          }}
        >
          <option value="">System Default</option>
          <For each={settingsState.outputDevices}>
            {(d) => (
              <option value={d.id}>
                {d.name}{d.isDefault ? " (default)" : ""}
              </option>
            )}
          </For>
        </select>
      </FormField>
      <div class="settings-hint">
        Changes take effect immediately during a call; saved otherwise.
      </div>
    </>
  );
};

export default AudioTab;
