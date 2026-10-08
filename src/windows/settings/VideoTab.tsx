import { Component, createSignal, onMount, For, Show } from "solid-js";
import FormField from "../../components/common/FormField";
import { settingsState, setSettingsState } from "../../stores/settings.store";
import { commands } from "../../ipc/commands";

/** One camera choice: the webview deviceId when the webview path lists
 *  it, and the label both paths resolve by. */
interface CameraOption {
  deviceId: string | null;
  label: string;
}

const VideoTab: Component = () => {
  // Plan §Failure 2 — the persisted selection lives on
  // `settingsState.selectedVideoDeviceId/Label` and on
  // `Preferences.videoDeviceId/videoDeviceLabel`. The LABEL is the
  // stable key — WebKit deviceIds are origin/data-store salted and
  // rotate across reinstalls.
  const [cameras, setCameras] = createSignal<CameraOption[]>([]);
  const [videoEnumError, setVideoEnumError] = createSignal<string | null>(null);
  const [native, setNative] = createSignal(false);

  // Where the backend owns the camera (native capture), the list comes
  // from the backend's device monitor and this window never opens the
  // camera: a second client of the device can change its capture format
  // under a running call (macOS AVCaptureDevice.activeFormat), and V4L2
  // allows one opener. The webview path needs one getUserMedia grant in
  // THIS window before labels populate; the temporary stream stops at once.
  async function refreshDevices(): Promise<void> {
    setVideoEnumError(null);
    const nativeAvailable = await commands
      .nativeVideoCaptureAvailable()
      .catch(() => false);
    setNative(nativeAvailable);
    if (nativeAvailable) {
      try {
        const devices = await commands.listNativeVideoDevices();
        setCameras(
          devices
            .map((d) => d.displayName)
            .filter((label) => label)
            .map((label) => ({ deviceId: null, label })),
        );
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        setVideoEnumError(msg);
        void commands.reportMediaCaptureError("settings-enumerate", msg);
      }
      return;
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        video: true,
        audio: false,
      });
      stream.getTracks().forEach((t) => t.stop());
    } catch (e) {
      const msg = e instanceof Error ? `${e.name}: ${e.message}` : String(e);
      setVideoEnumError(msg);
      void commands.reportMediaCaptureError("settings-enumerate", msg);
    }
    try {
      const devices = await navigator.mediaDevices.enumerateDevices();
      setCameras(
        devices
          .filter((d) => d.kind === "videoinput")
          .map((d) => ({
            deviceId: d.deviceId,
            label: d.label || `Camera ${d.deviceId.slice(0, 6)}`,
          })),
      );
    } catch (inner) {
      console.error("Failed to enumerate video devices:", inner);
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

  // The backend saves the choice and moves a running camera to it.
  function choose(option: CameraOption | null): void {
    setSettingsState("selectedVideoDeviceId", option?.deviceId ?? null);
    setSettingsState("selectedVideoDeviceLabel", option?.label ?? null);
    commands
      .setVideoDevice(option?.deviceId ?? null, option?.label ?? null)
      .catch((e) => {
        console.error("Failed to set the camera:", e);
      });
  }

  /** The option key: deviceId on the webview path, label on native. */
  const keyOf = (option: CameraOption): string =>
    option.deviceId ?? `label:${option.label}`;

  const selectedKey = (): string => {
    const id = settingsState.selectedVideoDeviceId;
    const label = settingsState.selectedVideoDeviceLabel;
    const match = cameras().find(
      (c) => (id !== null && c.deviceId === id) || (label !== null && c.label === label),
    );
    return match ? keyOf(match) : "";
  };

  return (
    <>
      <div class="settings-section-title">Camera</div>
      <FormField label="Camera Device">
        <select
          class="form-select"
          value={selectedKey()}
          onChange={(e) => {
            const key = e.currentTarget.value;
            choose(cameras().find((c) => keyOf(c) === key) ?? null);
          }}
        >
          <option value="">System Default</option>
          <For each={cameras()}>
            {(c) => <option value={keyOf(c)}>{c.label}</option>}
          </For>
        </select>
      </FormField>
      <Show when={cameras().length === 0 || videoEnumError()}>
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
        <Show
          when={native()}
          fallback="Selection applies to the next outgoing video call."
        >
          A camera already on in a call switches to the selection.
        </Show>{" "}
        Screen sharing uses the OS picker.
      </div>
    </>
  );
};

export default VideoTab;
