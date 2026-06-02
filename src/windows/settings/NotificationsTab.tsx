import { Component, createSignal, onMount, For } from "solid-js";
import FormField from "../../components/common/FormField";
import { settingsState } from "../../stores/settings.store";
import { handleSaveSettings } from "../../handlers/settings.handlers";
import { commands } from "../../ipc/commands";

const NotificationsTab: Component = () => {
  // Architecture §17.2 / §17.3 — Do Not Disturb + quiet hours.
  const [dnd, setDnd] = createSignal<boolean>(false);
  const [qhEnabled, setQhEnabled] = createSignal<boolean>(false);
  const [qhStart, setQhStart] = createSignal<number>(22);
  const [qhEnd, setQhEnd] = createSignal<number>(7);
  // Architecture §17.2 — IANA timezone for DST-aware quiet-hours
  // resolution. Defaults to the OS-resolved zone on first load via
  // `Intl.DateTimeFormat().resolvedOptions().timeZone` so the user
  // doesn't have to pick a zone before saving.
  const [qhTimezone, setQhTimezone] = createSignal<string>(
    Intl.DateTimeFormat().resolvedOptions().timeZone,
  );
  // Architecture §28.8 line 3220 — IP-privacy toggle for outgoing link previews.
  const [linkPreviewsEnabled, setLinkPreviewsEnabled] = createSignal<boolean>(true);

  onMount(() => {
    commands.getLinkPreviewsEnabled().then(setLinkPreviewsEnabled).catch((e) => {
      console.error("Failed to load link-previews setting:", e);
    });
    commands.getDoNotDisturb().then(setDnd).catch((e) => {
      console.error("Failed to load DND:", e);
    });
    commands.getQuietHours().then((qh) => {
      setQhEnabled(qh.enabled);
      setQhStart(qh.startHour);
      setQhEnd(qh.endHour);
      setQhTimezone(qh.timezone);
    }).catch((e) => {
      console.error("Failed to load quiet hours:", e);
    });
  });

  function handleToggle(key: keyof typeof settingsState): void {
    handleSaveSettings({ [key]: !settingsState[key] });
  }

  // Wave 12 W12.2 — slider for numeric settings (ringtone volume).
  function handleSliderChange(
    key: keyof typeof settingsState,
    value: number,
  ): void {
    handleSaveSettings({ [key]: value });
  }

  async function handleToggleDnd(): Promise<void> {
    const next = !dnd();
    setDnd(next);
    try {
      await commands.setDoNotDisturb(next);
    } catch (e) {
      console.error("Failed to set DND:", e);
      setDnd(!next);
    }
  }

  async function handleToggleLinkPreviews(): Promise<void> {
    const next = !linkPreviewsEnabled();
    setLinkPreviewsEnabled(next);
    try {
      await commands.setLinkPreviewsEnabled(next);
    } catch (e) {
      console.error("Failed to set link-previews:", e);
      setLinkPreviewsEnabled(!next);
    }
  }

  async function persistQuietHours(): Promise<void> {
    try {
      await commands.setQuietHours(qhEnabled(), qhStart(), qhEnd(), qhTimezone());
    } catch (e) {
      console.error("Failed to set quiet hours:", e);
    }
  }

  // Architecture §17.2 — IANA zone catalog from the OS, ordered
  // alphabetically. `Intl.supportedValuesOf` is shipped in every
  // WebView2 / WebKit / Gecko version Tauri 2 currently targets.
  const supportedTimezones = (): string[] => {
    const list = (Intl as unknown as { supportedValuesOf?: (key: string) => string[] })
      .supportedValuesOf?.("timeZone") ?? [];
    return [...list].sort();
  };

  return (
    <>
      <div class="settings-section-title">Notifications</div>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={settingsState.notifications}
          onChange={() => handleToggle("notifications")}
        />
        <span class="buddy-name">Enable Notifications</span>
      </label>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={settingsState.soundEnabled}
          onChange={() => handleToggle("soundEnabled")}
        />
        <span class="buddy-name">Sound Effects</span>
      </label>

      {/* Wave 12 W12.2 — call-specific audio gates. Ringtone is
          independent of message Sound Effects so the user can silence
          chat dings while still hearing the ring. */}
      <div class="settings-section-title">Calls</div>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={settingsState.ringtoneEnabled}
          onChange={() => handleToggle("ringtoneEnabled")}
        />
        <span class="buddy-name">Play ringtone on incoming calls</span>
      </label>
      <label class="settings-option">
        <span class="buddy-name">Ringtone volume</span>
        <input
          type="range"
          min="0"
          max="1"
          step="0.05"
          value={settingsState.ringtoneVolume}
          onInput={(e) =>
            handleSliderChange(
              "ringtoneVolume",
              Number(e.currentTarget.value),
            )
          }
        />
        <span class="settings-hint" style="min-width: 36px; text-align: right;">
          {Math.round(settingsState.ringtoneVolume * 100)}%
        </span>
      </label>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={settingsState.inCallDndAutoEnable}
          onChange={() => handleToggle("inCallDndAutoEnable")}
        />
        <span class="buddy-name">
          Suppress message notifications while in a call
        </span>
      </label>

      {/* Architecture §17.2 — Do Not Disturb suppresses every
          notification regardless of channel level, mention status, or
          quiet-hours window. */}
      <div class="settings-section-title">Do Not Disturb</div>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={dnd()}
          onChange={() => void handleToggleDnd()}
        />
        <span class="buddy-name">Suppress all notifications</span>
      </label>

      {/* Architecture §28.8 line 3220 — IP privacy. The OpenGraph fetch
          for outgoing link previews bypasses Veilid; disabling it stops
          third-party servers from learning this device's IP. */}
      <div class="settings-section-title">Link Previews</div>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={linkPreviewsEnabled()}
          onChange={() => void handleToggleLinkPreviews()}
        />
        <span class="buddy-name">
          Generate previews for URLs I post (reveals my IP to those sites)
        </span>
      </label>
      <div class="settings-hint">
        When enabled, this device fetches OpenGraph metadata directly from
        the destination site (the fetch bypasses Veilid). Receivers always
        re-validate your <code>EMBED_LINKS</code> permission before
        rendering the card, so disabling here only affects outbound previews.
      </div>

      {/* Architecture §17.2 — quiet hours suppress notifications
          during the configured local-time window. */}
      <div class="settings-section-title">Quiet Hours</div>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={qhEnabled()}
          onChange={(e) => {
            setQhEnabled(e.currentTarget.checked);
            void persistQuietHours();
          }}
        />
        <span class="buddy-name">Enable quiet hours</span>
      </label>
      <FormField label="Start hour (0–23)">
        <input
          class="form-input"
          type="number"
          min={0}
          max={23}
          value={qhStart()}
          disabled={!qhEnabled()}
          onChange={(e) => {
            const value = Math.max(0, Math.min(23, parseInt(e.currentTarget.value, 10) || 0));
            setQhStart(value);
            void persistQuietHours();
          }}
        />
      </FormField>
      <FormField label="End hour (0–23)">
        <input
          class="form-input"
          type="number"
          min={0}
          max={23}
          value={qhEnd()}
          disabled={!qhEnabled()}
          onChange={(e) => {
            const value = Math.max(0, Math.min(23, parseInt(e.currentTarget.value, 10) || 0));
            setQhEnd(value);
            void persistQuietHours();
          }}
        />
      </FormField>
      <FormField label="Timezone">
        <select
          class="form-select"
          value={qhTimezone()}
          disabled={!qhEnabled()}
          onChange={(e) => {
            setQhTimezone(e.currentTarget.value);
            void persistQuietHours();
          }}
        >
          <For each={supportedTimezones()}>
            {(zone) => <option value={zone}>{zone}</option>}
          </For>
        </select>
        <div class="settings-hint">
          DST transitions are honored automatically. Current local time:{" "}
          {new Date().toLocaleTimeString(undefined, {
            hour: "2-digit",
            minute: "2-digit",
            timeZone: qhTimezone(),
            timeZoneName: "short",
          })}
        </div>
      </FormField>
    </>
  );
};

export default NotificationsTab;
