import { Component, createSignal, onMount, For, Show } from "solid-js";
import AddDeviceModal from "../../components/settings/AddDeviceModal";
import { commands } from "../../ipc/commands";

const DevicesTab: Component = () => {
  // Architecture §28.4 — paired devices list. The pairing flow itself
  // lives inside `AddDeviceModal`; this tab only renders the existing
  // device list and the button that opens the modal.
  const [pairedDevices, setPairedDevices] = createSignal<{
    deviceId: string;
    devicePublicKey: string;
    displayName: string;
    pairedAt: number;
    unpairedAt?: number;
  }[]>([]);
  const [addDeviceOpen, setAddDeviceOpen] = createSignal(false);

  async function loadDevices(): Promise<void> {
    try {
      // Ensure the personal sync record exists before reading the
      // device list — first-time users won't have one yet.
      await commands.ensurePersonalSyncRecord().catch(() => undefined);
      const list = await commands.readPairedDevices();
      setPairedDevices(list.devices);
    } catch (e) {
      console.error("Failed to load paired devices:", e);
    }
  }

  onMount(() => {
    void loadDevices();
  });

  return (
    <>
      <div class="settings-section-title">Devices</div>
      <div class="settings-hint">
        Pair another device so it can read the same communities and
        direct messages. Each paired device keeps its own keys; the
        handshake uses a one-time 12-word code with a 5-minute expiry
        (architecture §28.4).
      </div>
      <div class="settings-button-row">
        <button
          class="form-btn-primary"
          type="button"
          onClick={() => setAddDeviceOpen(true)}
        >
          Add a device…
        </button>
      </div>

      <div class="settings-section-title">Paired devices</div>
      <Show
        when={pairedDevices().length > 0}
        fallback={<div class="settings-hint">No paired devices yet.</div>}
      >
        <For each={pairedDevices()}>
          {(device) => (
            <div class="settings-about-row">
              <span class="form-field-label">{device.displayName || "(unnamed)"}</span>
              <span class="settings-about-value">
                {device.unpairedAt
                  ? `Unpaired ${new Date(device.unpairedAt * 1000).toLocaleString()}`
                  : `Paired ${new Date(device.pairedAt * 1000).toLocaleString()}`}
              </span>
            </div>
          )}
        </For>
      </Show>

      <AddDeviceModal
        isOpen={addDeviceOpen()}
        onClose={() => {
          setAddDeviceOpen(false);
          void loadDevices();
        }}
        onPaired={() => void loadDevices()}
      />
    </>
  );
};

export default DevicesTab;
