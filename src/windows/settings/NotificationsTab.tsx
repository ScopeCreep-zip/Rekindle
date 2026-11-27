import { type Component, createSignal, onMount, For, Show, createUniqueId } from "solid-js";
import FormField from "../../components/common/FormField";
import { settingsState } from "../../stores/settings.store";
import { handleSaveSettings } from "../../actions/settings.actions";
import { commands } from "../../ipc/commands";
import type { NotificationContent, QuietHoursSettings } from "../../ipc/commands/types";

const NotificationsTab: Component = () => {
  const notificationDetailId = createUniqueId();
  const quietHoursStartId = createUniqueId();
  const quietHoursEndId = createUniqueId();
  const quietHoursTimezoneId = createUniqueId();
  // Architecture §17.2 / §17.3 — Do Not Disturb + quiet hours.
  const [dnd, setDnd] = createSignal<boolean>(false);
  // Quiet hours, including the IANA timezone for DST-aware resolution
  // (§17.2). `null` until get_quiet_hours answers: the box shows unticked, the
  // hour fields and zone show nothing, and all stay disabled, never a value
  // not read (they used to show 22, 7 and the OS zone; the stored row
  // defaults to UTC, `001_init.sql` quiet_hours_timezone). An edit made
  // before the answer would also have been overwritten by it (walk row ST22).
  const [qh, setQh] = createSignal<QuietHoursSettings | null>(null);
  // Architecture §28.8 line 3220 — IP-privacy toggle for outgoing link
  // previews. `null` until the stored value loads: the box shows unticked and
  // disabled, never a value it has not read (it used to start ticked).
  const [linkPreviewsEnabled, setLinkPreviewsEnabled] = createSignal<boolean | null>(null);
  // DND stays disabled until its stored value has loaded: an edit made
  // before the answer arrived was overwritten by it (walk row ST22).
  const [dndLoaded, setDndLoaded] = createSignal(false);

  onMount(() => {
    commands.getLinkPreviewsEnabled().then((on) => {
      setLinkPreviewsEnabled(on);
    }).catch((e) => {
      console.error("Failed to load link-previews setting:", e);
    });
    commands.getDoNotDisturb().then((on) => {
      setDnd(on);
      setDndLoaded(true);
    }).catch((e) => {
      console.error("Failed to load DND:", e);
    });
    commands.getQuietHours().then((stored) => {
      setQh(stored);
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
    const current = linkPreviewsEnabled();
    if (current === null) return;
    const next = !current;
    setLinkPreviewsEnabled(next);
    try {
      await commands.setLinkPreviewsEnabled(next);
    } catch (e) {
      console.error("Failed to set link-previews:", e);
      setLinkPreviewsEnabled(!next);
    }
  }

  // Applies one edit to the loaded quiet hours and saves them; an edit while
  // they are unknown does nothing (the controls are disabled then).
  async function editQuietHours(change: Partial<QuietHoursSettings>): Promise<void> {
    const current = qh();
    if (current === null) return;
    const next = { ...current, ...change };
    setQh(next);
    try {
      await commands.setQuietHours(next.enabled, next.startHour, next.endHour, next.timezone);
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
    // The stored zone is always listed: the row defaults to "UTC", which
    // Chromium's catalog omits, and a select without its value shows the
    // first zone instead, a value never read.
    const stored = qh()?.timezone;
    const zones = stored !== undefined && !list.includes(stored) ? [...list, stored] : [...list];
    return zones.sort();
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

      <FormField label="Notifications show" controlId={notificationDetailId}>
        <select
          id={notificationDetailId}
          class="form-select"
          value={settingsState.notificationContent}
          onChange={(e) =>
            handleSaveSettings({
              notificationContent: e.currentTarget.value as NotificationContent,
            })
          }
        >
          <option value="nameAndMessage">Name and message</option>
          <option value="nameOnly">Name only (sender or channel, no text)</option>
          <option value="nothing">No name or message</option>
        </select>
      </FormField>

      {/* Architecture §17.2 — Do Not Disturb suppresses every
          notification regardless of channel level, mention status, or
          quiet-hours window. */}
      <div class="settings-section-title">Do Not Disturb</div>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={dnd()}
          disabled={!dndLoaded()}
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
          checked={linkPreviewsEnabled() === true}
          disabled={linkPreviewsEnabled() === null}
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
          checked={qh()?.enabled === true}
          disabled={qh() === null}
          onChange={(e) => void editQuietHours({ enabled: e.currentTarget.checked })}
        />
        <span class="buddy-name">Enable quiet hours</span>
      </label>
      <FormField label="Start hour (0–23)" controlId={quietHoursStartId}>
        <input
          id={quietHoursStartId}
          class="form-input"
          type="number"
          min={0}
          max={23}
          value={qh()?.startHour ?? ""}
          disabled={qh()?.enabled !== true}
          onChange={(e) => {
            const value = Math.max(0, Math.min(23, parseInt(e.currentTarget.value, 10) || 0));
            void editQuietHours({ startHour: value });
          }}
        />
      </FormField>
      <FormField label="End hour (0–23)" controlId={quietHoursEndId}>
        <input
          id={quietHoursEndId}
          class="form-input"
          type="number"
          min={0}
          max={23}
          value={qh()?.endHour ?? ""}
          disabled={qh()?.enabled !== true}
          onChange={(e) => {
            const value = Math.max(0, Math.min(23, parseInt(e.currentTarget.value, 10) || 0));
            void editQuietHours({ endHour: value });
          }}
        />
      </FormField>
      <FormField label="Timezone" controlId={quietHoursTimezoneId}>
        <select
          id={quietHoursTimezoneId}
          class="form-select"
          value={qh()?.timezone ?? ""}
          disabled={qh()?.enabled !== true}
          onChange={(e) => void editQuietHours({ timezone: e.currentTarget.value })}
        >
          <Show when={qh() === null}>
            <option value="" />
          </Show>
          <For each={supportedTimezones()}>
            {(zone) => <option value={zone}>{zone}</option>}
          </For>
        </select>
        <div class="settings-hint">
          DST transitions are honored automatically.
          <Show when={qh()}>
            {(loaded) => (
              <>
                {" "}Current local time:{" "}
                {new Date().toLocaleTimeString(undefined, {
                  hour: "2-digit",
                  minute: "2-digit",
                  timeZone: loaded().timezone,
                  timeZoneName: "short",
                })}
              </>
            )}
          </Show>
        </div>
      </FormField>
    </>
  );
};

export default NotificationsTab;
