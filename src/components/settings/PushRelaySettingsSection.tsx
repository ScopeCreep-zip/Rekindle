import { Component, For, Show, createSignal, onMount } from "solid-js";
import { commands } from "../../ipc/commands";
import FormField from "../common/FormField";

/// Mobile push relay configuration (architecture §17.3 Tier 3).
///
/// On desktop this is mainly diagnostic — a real device would feed in
/// its FCM/APNs token. We expose it here so users running their own
/// `rekindle-push-relay` daemon can register the desktop client as a
/// test consumer (platform="self") and verify wake plumbing.
const PushRelaySettingsSection: Component = () => {
  const [registrations, setRegistrations] =
    createSignal<{ pseudonym: string; platform: string; recordKeys: string[] }[]>([]);
  const [relayPseudonym, setRelayPseudonym] = createSignal("");
  const [deviceToken, setDeviceToken] = createSignal("");
  const [platform, setPlatform] = createSignal<"fcm" | "apns" | "self">("self");
  const [recordKeysInput, setRecordKeysInput] = createSignal("");
  const [busy, setBusy] = createSignal(false);

  async function refresh(): Promise<void> {
    const list = await commands.listPushRelayRegistrations();
    setRegistrations(
      list.map(([pseudonym, plat, json]) => ({
        pseudonym,
        platform: plat,
        recordKeys: safeParse(json),
      })),
    );
  }

  function safeParse(json: string): string[] {
    try {
      const parsed = JSON.parse(json);
      return Array.isArray(parsed) ? parsed.filter((s) => typeof s === "string") : [];
    } catch {
      return [];
    }
  }

  onMount(refresh);

  async function handleRegister(): Promise<void> {
    const pseudonym = relayPseudonym().trim();
    const token = deviceToken().trim();
    const keys = recordKeysInput()
      .split(/[\s,]+/)
      .map((k) => k.trim())
      .filter(Boolean);
    if (!pseudonym || !token || keys.length === 0) return;
    setBusy(true);
    try {
      await commands.registerWithPushRelay(pseudonym, token, platform(), keys);
      setRelayPseudonym("");
      setDeviceToken("");
      setRecordKeysInput("");
      await refresh();
    } finally {
      setBusy(false);
    }
  }

  async function handleUnregister(pseudonym: string): Promise<void> {
    setBusy(true);
    try {
      await commands.unregisterWithPushRelay(pseudonym);
      await refresh();
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <div class="settings-section-title">Notification delivery while suspended</div>
      <details class="settings-details" id="mobile-tier-explanation">
        <summary>How notifications reach a backgrounded device</summary>
        <p>
          <strong>Tier 1 — Local watch</strong> (lowest latency, this device only):
          your device watches the DHT directly while running. No external service
          involved.
        </p>
        <p>
          <strong>Tier 2 — Friend relay</strong> (peer-to-peer, no token):
          friends optionally watch your records and forward updates over Veilid
          while your device is suspended.
        </p>
        <p>
          <strong>Tier 3 — Push relay daemon</strong> (mobile background): a
          separate <code>rekindle-push-relay</code> daemon watches DHT records
          on your behalf and sends a content-free wake to FCM / APNs. The
          daemon learns request timing and watched record keys, but never reads
          message content (which stays MEK-encrypted at rest).
        </p>
        <p class="settings-hint">
          Tier 3 should be opt-in, not the default. Most users will be served
          by Tier 1 alone; Tier 2 covers brief background windows; Tier 3 is
          only useful for fully-offline mobile clients.
        </p>
      </details>

      <div class="settings-section-title">Push relay registration</div>
      <div class="settings-hint">
        Discovery is out-of-band — paste the relay's pseudonym and your push
        token below. Register the keys you want it to watch (comma or newline
        separated). Tokens are stored locally and never displayed in
        plaintext after registration.
      </div>

      <FormField label="Relay pseudonym (hex)">
        <input
          type="text"
          class="form-input"
          value={relayPseudonym()}
          onInput={(e) => setRelayPseudonym(e.currentTarget.value)}
          placeholder="64-char Ed25519 hex"
          aria-describedby="mobile-tier-explanation"
        />
      </FormField>
      <FormField label="Device push token">
        <input
          type="text"
          class="form-input"
          value={deviceToken()}
          onInput={(e) => setDeviceToken(e.currentTarget.value)}
          placeholder="FCM registration id / APNs device token / opaque self id"
        />
      </FormField>
      <FormField label="Platform">
        <select
          class="form-input"
          value={platform()}
          onChange={(e) => setPlatform(e.currentTarget.value as "fcm" | "apns" | "self")}
        >
          <option value="self">self-hosted</option>
          <option value="fcm">FCM (Android)</option>
          <option value="apns">APNs (iOS)</option>
        </select>
      </FormField>
      <FormField label="Record keys to watch">
        <textarea
          class="form-input"
          rows={3}
          value={recordKeysInput()}
          onInput={(e) => setRecordKeysInput(e.currentTarget.value)}
          placeholder="One per line or comma-separated"
        />
      </FormField>
      <div class="form-field-row">
        <button
          class="form-btn-primary"
          disabled={busy()}
          onClick={() => handleRegister()}
        >
          Register
        </button>
      </div>

      <div class="settings-section-title">Active registrations</div>
      <Show
        when={registrations().length > 0}
        fallback={<div class="settings-hint">No relay registrations.</div>}
      >
        <For each={registrations()}>
          {(reg) => (
            <div class="push-relay-row">
              <div class="push-relay-meta">
                <span class="buddy-name">{reg.pseudonym.slice(0, 20)}…</span>
                <span class="push-relay-platform">{reg.platform}</span>
                <span class="push-relay-watch-count">
                  {reg.recordKeys.length} record(s)
                </span>
              </div>
              <button
                class="form-btn-secondary"
                disabled={busy()}
                onClick={() => handleUnregister(reg.pseudonym)}
              >
                Unregister
              </button>
            </div>
          )}
        </For>
      </Show>
    </>
  );
};

export default PushRelaySettingsSection;
