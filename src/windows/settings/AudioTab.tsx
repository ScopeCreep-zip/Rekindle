import { type Component, For, onMount, Show } from "solid-js";
import FormField from "../../components/common/FormField";
import SettingsSelect, { type SettingsSelectOption } from "../../components/common/SettingsSelect";
import type { AudioDeviceInfo } from "../../ipc/commands";
import { commands } from "../../ipc/commands";
import { setSettingsState, settingsState } from "../../stores/settings.store";

/** A device list's choices: "" is the system default, then each device,
 *  the one the system uses marked "(default)". */
const deviceOptions = (devices: AudioDeviceInfo[]): SettingsSelectOption[] => [
  { value: "", label: "System Default" },
  ...devices.map((d) => ({ value: d.id, label: `${d.name}${d.isDefault ? " (default)" : ""}` })),
];

const AudioTab: Component = () => {
  // Plan §Failure 1 — load enumerated audio devices + the persisted
  // selection when the audio tab opens. Which device a call opens when the
  // saved one is missing is the backend's decision (plan C7.24).

  onMount(() => {
    commands
      .listAudioDevices()
      .then((d) => {
        setSettingsState("inputDevices", d.inputDevices);
        setSettingsState("outputDevices", d.outputDevices);
      })
      .catch((e) => {
        console.error("Failed to enumerate audio devices:", e);
      });
    commands
      .getPreferences()
      .then((prefs) => {
        setSettingsState("selectedInputDevice", prefs.inputDevice);
        setSettingsState("selectedOutputDevice", prefs.outputDevice);
        setSettingsState("inputChannels", prefs.inputChannels ?? {});
      })
      .catch((e) => {
        console.error("Failed to load device preferences:", e);
      });
  });

  /** The input device the channel choice applies to: the selected one, or
   *  the system default. */
  const inputDevice = () =>
    settingsState.inputDevices.find((d) =>
      settingsState.selectedInputDevice ? d.id === settingsState.selectedInputDevice : d.isDefault
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
        settingsState.selectedOutputDevice
      );
    } catch (e) {
      console.error("Failed to persist audio device selection:", e);
    }
  }

  return (
    <>
      <div class="settings-section-title">Audio Devices</div>
      <FormField label="Input Device" labelId="settings-input-device-label">
        <SettingsSelect
          labelledBy="settings-input-device-label"
          value={settingsState.selectedInputDevice ?? ""}
          onChange={(value) => {
            const next = value || null;
            setSettingsState("selectedInputDevice", next);
            void persistAudioSelection();
          }}
          options={deviceOptions(settingsState.inputDevices)}
        />
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
      <FormField label="Output Device" labelId="settings-output-device-label">
        <SettingsSelect
          labelledBy="settings-output-device-label"
          value={settingsState.selectedOutputDevice ?? ""}
          onChange={(value) => {
            const next = value || null;
            setSettingsState("selectedOutputDevice", next);
            void persistAudioSelection();
          }}
          options={deviceOptions(settingsState.outputDevices)}
        />
      </FormField>
      <div class="settings-hint">
        Changes take effect immediately during a call; saved otherwise.
      </div>
    </>
  );
};

export default AudioTab;
