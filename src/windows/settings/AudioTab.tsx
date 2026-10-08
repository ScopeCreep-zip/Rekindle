import { Component, onMount, For, Show } from "solid-js";
import FormField from "../../components/common/FormField";
import { settingsState, setSettingsState } from "../../stores/settings.store";
import { commands } from "../../ipc/commands";

const AudioTab: Component = () => {
  // Plan §Failure 1 — load enumerated audio devices + the persisted
  // selection when the audio tab opens. Which device a call opens when the
  // saved one is missing is the backend's decision (plan C7.24).

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
      setSettingsState("inputChannels", prefs.inputChannels ?? {});
    }).catch((e) => {
      console.error("Failed to load device preferences:", e);
    });
  });

  /** The input device the channel choice applies to: the selected one, or
   *  the system default. */
  const inputDevice = () =>
    settingsState.inputDevices.find((d) =>
      settingsState.selectedInputDevice
        ? d.id === settingsState.selectedInputDevice
        : d.isDefault,
    );

  /** Channel `index` is captured: listed, or no choice saved (all). */
  const channelOn = (device: string, index: number) => {
    const picked = settingsState.inputChannels[device];
    return !picked || picked.includes(index);
  };

  async function toggleChannel(device: string, count: number, index: number): Promise<void> {
    const all = Array.from({ length: count }, (_, i) => i);
    const current = settingsState.inputChannels[device] ?? all;
    const next = current.includes(index)
      ? current.filter((c) => c !== index)
      : [...current, index].sort((a, b) => a - b);
    if (next.length === 0) return; // at least one input stays on
    const saved = next.length === count ? [] : next;
    try {
      await commands.setInputChannels(device, saved);
      setSettingsState("inputChannels", device, saved.length ? saved : undefined);
    } catch (e) {
      console.error("Failed to save input channels:", e);
    }
  }

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
      <Show when={(inputDevice()?.channels ?? 0) > 1 && inputDevice()}>
        {(device) => (
          <FormField label="Input Channels">
            <For each={Array.from({ length: device().channels }, (_, i) => i)}>
              {(index) => (
                <label class="settings-option">
                  <input
                    type="checkbox"
                    checked={channelOn(device().id, index)}
                    onChange={() => void toggleChannel(device().id, device().channels, index)}
                  />
                  <span class="buddy-name">Input {index + 1}</span>
                </label>
              )}
            </For>
          </FormField>
        )}
      </Show>
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
