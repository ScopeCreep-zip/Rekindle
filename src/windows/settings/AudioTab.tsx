import { Component, onMount, For, Show } from "solid-js";
import FormField from "../../components/common/FormField";
import { settingsState, setSettingsState } from "../../stores/settings.store";
import { commands } from "../../ipc/commands";
import { addToast } from "../../stores/toast.store";
import { errorMessage } from "../../utils/error";
import { loadVoiceStatus } from "../../handlers/voice_status.handlers";
import { VoiceEndLine } from "../../components/voice/VoiceEndNotices";

const AudioTab: Component = () => {
  // Plan §Failure 1: the Host's audio devices, its voice status and the
  // persisted selection when the audio tab opens; the device lists and the
  // status then follow the Host's events (`voice_status.handlers.ts`).
  // Which device a call opens when the saved one is missing is the Host's
  // decision (plan C7.24); each end's line says what it did (E4.1 S5).

  onMount(() => {
    void loadVoiceStatus();
    commands.getPreferences().then((prefs) => {
      setSettingsState("selectedInputDevice", prefs.inputDevice);
      setSettingsState("selectedOutputDevice", prefs.outputDevice);
      setSettingsState("inputChannels", prefs.inputChannels ?? {});
    }).catch((e) => addToast(errorMessage(e), "error"));
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
      addToast(errorMessage(e), "error");
    }
  }

  async function persistAudioSelection(): Promise<void> {
    try {
      await commands.setAudioDevices(
        settingsState.selectedInputDevice,
        settingsState.selectedOutputDevice,
      );
    } catch (e) {
      addToast(errorMessage(e), "error");
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
        <VoiceEndLine end="input" />
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
        <VoiceEndLine end="output" />
      </FormField>
      <div class="settings-hint">
        Changes take effect immediately during a call; saved otherwise.
      </div>
    </>
  );
};

export default AudioTab;
