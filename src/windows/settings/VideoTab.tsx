import { Component, createSignal, onMount, For, Show } from "solid-js";
import FormField from "../../components/common/FormField";
import { settingsState, setSettingsState } from "../../stores/settings.store";
import { commands } from "../../ipc/commands";

const VideoTab: Component = () => {
  // Plan §Failure 2 — WebView-enumerated camera devices. The list is
  // UI-only (not persisted); the persisted selection lives on
  // `settingsState.selectedVideoDeviceId/Label` and on
  // `Preferences.videoDeviceId/videoDeviceLabel`. The LABEL is the
  // stable key — WebKit deviceIds are origin/data-store salted and
  // rotate across reinstalls.
  const [videoDevices, setVideoDevices] = createSignal<MediaDeviceInfo[]>([]);
  const [videoEnumError, setVideoEnumError] = createSignal<string | null>(null);
  /** Native-backend camera labels absent from the webview list —
   *  selected by label (no WebKit deviceId exists for them). */
  const [nativeOnlyLabels, setNativeOnlyLabels] = createSignal<string[]>([]);

  // WebView-side enumeration: getUserMedia must succeed once in THIS
  // window before labels populate; request a temporary stream,
  // enumerate, then immediately stop it. Failures show the REAL
  // exception text and reach the terminal log via the capture-error
  // bridge — a silent "System Default only" list is undiagnosable.
  async function refreshDevices(): Promise<void> {
    setVideoEnumError(null);
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        video: true,
        audio: false,
      });
      stream.getTracks().forEach((t) => t.stop());
      const devices = await navigator.mediaDevices.enumerateDevices();
      setVideoDevices(devices.filter((d) => d.kind === "videoinput"));
    } catch (e) {
      const msg = e instanceof Error ? `${e.name}: ${e.message}` : String(e);
      setVideoEnumError(msg);
      void commands.reportMediaCaptureError("settings-enumerate", msg);
      try {
        const devices = await navigator.mediaDevices.enumerateDevices();
        setVideoDevices(devices.filter((d) => d.kind === "videoinput"));
      } catch (inner) {
        console.error("Failed to enumerate video devices:", inner);
      }
    }
    // Backend-native device list (Linux GStreamer DeviceMonitor):
    // labels the webview can't see while getUserMedia is blocked or a
    // native session owns the camera. Same labels getUserMedia would
    // report — selection persists by label either way.
    try {
      const native = await commands.listNativeVideoDevices();
      const seen = new Set(videoDevices().map((d) => d.label));
      setNativeOnlyLabels(
        native.map((d) => d.displayName).filter((l) => l && !seen.has(l)),
      );
    } catch {
      // Off-Linux / probe failed — webview list only.
    }
  }

  onMount(() => {
    void refreshDevices();
    commands
      .getPreferences()
      .then((prefs) => {
        setSettingsState("selectedVideoDeviceId", prefs.videoDeviceId);
        setSettingsState("selectedVideoDeviceLabel", prefs.videoDeviceLabel);
      })
      .catch((e) => {
        console.error("Failed to load video preferences:", e);
      });
  });

  // Persist BOTH id and label — `VideoCallPanel.startCamera()` resolves
  // id-first, then label, then default. The full Preferences struct is
  // round-tripped (the store has no partial-update command).
  async function persistVideoSelection(): Promise<void> {
    try {
      const prefs = await commands.getPreferences();
      prefs.videoDeviceId = settingsState.selectedVideoDeviceId;
      prefs.videoDeviceLabel = settingsState.selectedVideoDeviceLabel;
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
          value={
            settingsState.selectedVideoDeviceId ??
            (settingsState.selectedVideoDeviceLabel &&
            nativeOnlyLabels().includes(settingsState.selectedVideoDeviceLabel)
              ? `native:${settingsState.selectedVideoDeviceLabel}`
              : "")
          }
          onChange={(e) => {
            const raw = e.currentTarget.value;
            if (raw.startsWith("native:")) {
              // Native-backend device — no WebKit deviceId exists; the
              // label is the persisted key both paths resolve by.
              setSettingsState("selectedVideoDeviceId", null);
              setSettingsState("selectedVideoDeviceLabel", raw.slice("native:".length));
              void persistVideoSelection();
              return;
            }
            const next = raw || null;
            const label = next
              ? (videoDevices().find((d) => d.deviceId === next)?.label ?? null)
              : null;
            setSettingsState("selectedVideoDeviceId", next);
            setSettingsState("selectedVideoDeviceLabel", label);
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
          <For each={nativeOnlyLabels()}>
            {(label) => <option value={`native:${label}`}>{label}</option>}
          </For>
        </select>
      </FormField>
      <Show when={videoDevices().length === 0 || videoEnumError()}>
        <button
          type="button"
          class="form-button"
          onClick={() => void refreshDevices()}
        >
          Request camera access / refresh devices
        </button>
      </Show>
      <Show when={videoEnumError()}>
        <div class="settings-hint">
          Camera access failed: {videoEnumError()} — the device list stays
          empty until access succeeds (also logged to the app terminal).
        </div>
      </Show>
      <div class="settings-hint">
        Selection applies to the next outgoing video call. Screen sharing uses the OS picker.
      </div>
    </>
  );
};

export default VideoTab;
