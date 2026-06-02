import { Component, createSignal, onMount, For, Show } from "solid-js";
import FormField from "../../components/common/FormField";
import { settingsState, setSettingsState } from "../../stores/settings.store";
import { commands } from "../../ipc/commands";

const VideoTab: Component = () => {
  // Plan §Failure 2 — WebView-enumerated camera devices. The list is
  // UI-only (not persisted); the persisted selection lives on
  // `settingsState.selectedVideoDeviceId` and on `Preferences.videoDeviceId`.
  const [videoDevices, setVideoDevices] = createSignal<MediaDeviceInfo[]>([]);
  const [videoEnumError, setVideoEnumError] = createSignal<string | null>(null);

  // Plan §Failure 2 — enumerate cameras + load persisted selection on
  // mount. WebView-side enumeration: getUserMedia must succeed once
  // before labels are populated; we request a temporary stream,
  // enumerate, then immediately stop it.
  onMount(() => {
    setVideoEnumError(null);
    void (async () => {
      try {
        const stream = await navigator.mediaDevices.getUserMedia({ video: true, audio: false });
        stream.getTracks().forEach((t) => t.stop());
        const devices = await navigator.mediaDevices.enumerateDevices();
        setVideoDevices(devices.filter((d) => d.kind === "videoinput"));
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        setVideoEnumError(msg);
        try {
          const devices = await navigator.mediaDevices.enumerateDevices();
          setVideoDevices(devices.filter((d) => d.kind === "videoinput"));
        } catch (inner) {
          console.error("Failed to enumerate video devices:", inner);
        }
      }
    })();
    commands.getPreferences().then((prefs) => {
      setSettingsState("selectedVideoDeviceId", prefs.videoDeviceId);
    }).catch((e) => {
      console.error("Failed to load video preferences:", e);
    });
  });

  // Plan §Failure 2 — write `videoDeviceId` to the Preferences store.
  // Read by `VideoCallPanel.startCamera()` when a call begins. The full
  // Preferences struct is round-tripped (the store has no partial-update
  // command) — same shape as the rest of the settings tab.
  async function persistVideoSelection(): Promise<void> {
    try {
      const prefs = await commands.getPreferences();
      prefs.videoDeviceId = settingsState.selectedVideoDeviceId;
      await commands.setPreferences(prefs);
    } catch (e) {
      console.error("Failed to persist video device selection:", e);
    }
  }

  return (
    <>
      <div class="settings-section-title">Camera</div>
      <FormField label="Camera Device">
        <select
          class="form-select"
          value={settingsState.selectedVideoDeviceId ?? ""}
          onChange={(e) => {
            const next = e.currentTarget.value || null;
            setSettingsState("selectedVideoDeviceId", next);
            void persistVideoSelection();
          }}
        >
          <option value="">System Default</option>
          <For each={videoDevices()}>
            {(d) => (
              <option value={d.deviceId}>
                {d.label || `Camera ${d.deviceId.slice(0, 6)}`}
              </option>
            )}
          </For>
        </select>
      </FormField>
      <Show when={videoEnumError()}>
        <div class="settings-hint">
          Camera permission denied — labels will be hidden until access is granted.
        </div>
      </Show>
      <div class="settings-hint">
        Selection applies to the next outgoing video call. Screen sharing uses the OS picker.
      </div>
    </>
  );
};

export default VideoTab;
