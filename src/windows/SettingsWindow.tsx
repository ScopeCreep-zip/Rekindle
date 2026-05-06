import { Component, createSignal, createEffect, For, Show, onMount, onCleanup } from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import Titlebar from "../components/titlebar/Titlebar";
import Avatar from "../components/common/Avatar";
import FormField from "../components/common/FormField";
import RelaySettingsSection from "../components/settings/RelaySettingsSection";
import PushRelaySettingsSection from "../components/settings/PushRelaySettingsSection";
import AddDeviceModal from "../components/settings/AddDeviceModal";
import { settingsState, setSettingsState } from "../stores/settings.store";
import { authState, setAuthState } from "../stores/auth.store";
import {
  handleLoadSettings,
  handleSaveSettings,
  handleSetAvatar,
  handleCheckForUpdates,
} from "../handlers/settings.handlers";
import { commands } from "../ipc/commands";
import { hydrateState } from "../ipc/hydrate";
import { fetchAvatarUrl } from "../ipc/avatar";

function getInitialTab(): SettingsTab {
  const params = new URLSearchParams(window.location.search);
  const tab = params.get("tab");
  const valid: SettingsTab[] = ["profile", "application", "notifications", "audio", "video", "privacy", "devices", "mobile", "about"];
  if (tab && valid.includes(tab as SettingsTab)) {
    return tab as SettingsTab;
  }
  return "profile";
}

type SettingsTab = "profile" | "application" | "notifications" | "audio" | "video" | "privacy" | "devices" | "mobile" | "about";

const TAB_LABELS: { id: SettingsTab; label: string }[] = [
  { id: "profile", label: "Profile" },
  { id: "application", label: "Application" },
  { id: "notifications", label: "Notifications" },
  { id: "audio", label: "Audio" },
  { id: "video", label: "Video" },
  { id: "privacy", label: "Privacy" },
  { id: "devices", label: "Devices" },
  { id: "mobile", label: "Mobile" },
  { id: "about", label: "About" },
];

const SettingsWindow: Component = () => {
  const [activeTab, setActiveTab] = createSignal<SettingsTab>(getInitialTab());
  const [nameInput, setNameInput] = createSignal("");
  const [statusMsgInput, setStatusMsgInput] = createSignal("");
  const [checkingUpdates, setCheckingUpdates] = createSignal(false);
  const [updateResult, setUpdateResult] = createSignal<string | null>(null);
  const [blockedUsers, setBlockedUsers] = createSignal<{ publicKey: string; displayName: string; blockedAt: number }[]>([]);
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

  // Plan §Failure 2 — WebView-enumerated camera devices for the Video tab.
  // The list is UI-only (not persisted); the persisted selection lives on
  // `settingsState.selectedVideoDeviceId` and on `Preferences.videoDeviceId`.
  const [videoDevices, setVideoDevices] = createSignal<MediaDeviceInfo[]>([]);
  const [videoEnumError, setVideoEnumError] = createSignal<string | null>(null);

  let unlistenSwitchTab: Promise<UnlistenFn> | undefined;

  onMount(() => {
    hydrateState().then(() => {
      setNameInput(authState.displayName ?? "");
    });
    handleLoadSettings();

    unlistenSwitchTab = listen<string>("settings-switch-tab", (event) => {
      const valid: SettingsTab[] = ["profile", "application", "notifications", "audio", "video", "privacy", "devices", "mobile", "about"];
      if (valid.includes(event.payload as SettingsTab)) {
        setActiveTab(event.payload as SettingsTab);
      }
    });
  });

  onCleanup(() => {
    unlistenSwitchTab?.then((unlisten) => unlisten());
  });

  // Load blocked users when privacy tab is selected
  createEffect(() => {
    if (activeTab() === "privacy") {
      commands.getBlockedUsers().then(setBlockedUsers).catch((e) => {
        console.error("Failed to load blocked users:", e);
      });
    }
  });

  // Load paired devices when devices tab is selected.
  createEffect(() => {
    if (activeTab() === "devices") {
      void loadDevices();
    }
  });

  // Plan §Failure 1 — load enumerated audio devices + the persisted
  // selection when the audio tab opens. Mirrors the load-on-tab pattern
  // used for notifications below. The voice engine's `resolve_device`
  // (crates/rekindle-voice/src/device.rs:38) matches by `cpal::Device::name()`
  // and falls back to system default if the saved id no longer exists.
  createEffect(() => {
    if (activeTab() === "audio") {
      commands.listAudioDevices().then((d) => {
        setSettingsState("inputDevices", d.inputDevices);
        setSettingsState("outputDevices", d.outputDevices);
      }).catch((e) => {
        console.error("Failed to enumerate audio devices:", e);
      });
      commands.getPreferences().then((prefs) => {
        setSettingsState("selectedInputDevice", prefs.inputDevice);
        setSettingsState("selectedOutputDevice", prefs.outputDevice);
      }).catch((e) => {
        console.error("Failed to load device preferences:", e);
      });
    }
  });

  // Plan §Failure 2 — enumerate cameras + load persisted selection when
  // the video tab opens. WebView-side enumeration: getUserMedia must
  // succeed once before labels are populated; we request a temporary
  // stream, enumerate, then immediately stop it.
  createEffect(() => {
    if (activeTab() !== "video") return;
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

  // Load DND + quiet hours when notifications tab is selected.
  createEffect(() => {
    if (activeTab() === "notifications") {
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
    }
  });

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

  function handleToggle(key: keyof typeof settingsState): void {
    handleSaveSettings({ [key]: !settingsState[key] });
  }

  function handleSaveName(): void {
    const name = nameInput().trim();
    if (name && name !== authState.displayName) {
      setAuthState("displayName", name);
      commands.setNickname(name).catch((e) => {
        console.error("Failed to set nickname:", e);
      });
    }
  }

  function handleSaveStatusMessage(): void {
    const msg = statusMsgInput().trim();
    commands.setStatusMessage(msg).catch((e) => {
      console.error("Failed to set status message:", e);
    });
  }

  async function handleAvatarUpload(): Promise<void> {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/png, image/jpeg, image/gif";
    input.style.display = "none";
    document.body.appendChild(input);
    input.onchange = async () => {
      const file = input.files?.[0];
      document.body.removeChild(input);
      if (!file) return;
      const arrayBuffer = await file.arrayBuffer();
      const bytes = Array.from(new Uint8Array(arrayBuffer));
      await handleSetAvatar(bytes);
      // Refresh avatar in store after upload
      if (authState.publicKey) {
        const avatarUrl = await fetchAvatarUrl(authState.publicKey);
        setAuthState("avatarUrl", avatarUrl);
      }
    };
    input.click();
  }

  async function handleCheckUpdates(): Promise<void> {
    setCheckingUpdates(true);
    setUpdateResult(null);
    const available = await handleCheckForUpdates();
    if (available) {
      setUpdateResult("Update available! Restart to apply.");
    } else {
      setUpdateResult("You are on the latest version.");
    }
    setCheckingUpdates(false);
  }

  function renderProfile() {
    return (
      <>
        <div class="settings-section-title">Avatar</div>
        <div class="avatar-upload-section">
          <Avatar displayName={authState.displayName ?? "?"} size={64} avatarUrl={authState.avatarUrl ?? undefined} />
          <button class="avatar-upload-btn" onClick={handleAvatarUpload}>
            Change Avatar
          </button>
          <span class="avatar-upload-hint">PNG, JPEG, or GIF (max 256KB)</span>
        </div>
        <div class="settings-section-title">Display Name</div>
        <FormField>
          <div class="form-field-row">
            <input
              class="form-input"
              type="text"
              value={nameInput()}
              onInput={(e: InputEvent) => setNameInput((e.target as HTMLInputElement).value)}
              onKeyDown={(e: KeyboardEvent) => { if (e.key === "Enter") handleSaveName(); }}
            />
            <button class="form-btn-primary" onClick={handleSaveName}>Save</button>
          </div>
        </FormField>
        <div class="settings-section-title">Status Message</div>
        <FormField>
          <div class="form-field-row">
            <input
              class="form-input"
              type="text"
              placeholder="What's on your mind?"
              value={statusMsgInput()}
              onInput={(e: InputEvent) => setStatusMsgInput((e.target as HTMLInputElement).value)}
              onKeyDown={(e: KeyboardEvent) => { if (e.key === "Enter") handleSaveStatusMessage(); }}
            />
            <button class="form-btn-primary" onClick={handleSaveStatusMessage}>Save</button>
          </div>
        </FormField>
      </>
    );
  }

  function renderApplication() {
    return (
      <>
        <div class="settings-section-title">Startup</div>
        <label class="settings-option">
          <input
            type="checkbox"
            checked={settingsState.autoStart}
            onChange={() => handleToggle("autoStart")}
          />
          <span class="buddy-name">Start with System</span>
        </label>
        <label class="settings-option">
          <input
            type="checkbox"
            checked={settingsState.startMinimized}
            onChange={() => handleToggle("startMinimized")}
          />
          <span class="buddy-name">Start Minimized</span>
        </label>
        <div class="settings-section-title">Game Detection</div>
        <label class="settings-option">
          <input
            type="checkbox"
            checked={settingsState.showGameActivity}
            onChange={() => handleToggle("showGameActivity")}
          />
          <span class="buddy-name">Show Game Activity</span>
        </label>
        <div class="settings-section-title">Auto-Away</div>
        <FormField label="Go away after inactivity">
          <select
            class="form-select"
            value={settingsState.autoAwayMinutes}
            onChange={(e) =>
              handleSaveSettings({ autoAwayMinutes: parseInt(e.currentTarget.value) })
            }
          >
            <option value={0}>Disabled</option>
            <option value={5}>5 minutes</option>
            <option value={10}>10 minutes</option>
            <option value={15}>15 minutes</option>
            <option value={30}>30 minutes</option>
            <option value={60}>1 hour</option>
          </select>
        </FormField>
      </>
    );
  }

  function renderNotifications() {
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

  function renderAudio() {
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
  }

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

  function renderVideo() {
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
  }

  async function handleUnblock(publicKey: string): Promise<void> {
    try {
      await commands.unblockUser(publicKey);
      setBlockedUsers((prev) => prev.filter((u) => u.publicKey !== publicKey));
    } catch (e) {
      console.error("Failed to unblock user:", e);
    }
  }

  function renderPrivacy() {
    return (
      <>
        <div class="settings-section-title">Privacy</div>
        <FormField label="Public Key">
          <div class="profile-key-display">{authState.publicKey ?? "Not logged in"}</div>
        </FormField>
        <div class="settings-section-title">Identity</div>
        <div class="form-field-row">
          <button class="form-btn-secondary" disabled>Export Identity</button>
          <button class="form-btn-secondary" disabled>Import Identity</button>
        </div>
        <div class="settings-hint">Identity export/import requires Stronghold integration.</div>
        <div class="settings-section-title">Blocked Users</div>
        <Show when={blockedUsers().length > 0} fallback={
          <div class="settings-hint">No blocked users.</div>
        }>
          <For each={blockedUsers()}>
            {(user) => (
              <div class="blocked-user-item">
                <span class="buddy-name">{user.displayName || user.publicKey.slice(0, 12) + "..."}</span>
                <button class="form-btn-secondary" onClick={() => handleUnblock(user.publicKey)}>
                  Unblock
                </button>
              </div>
            )}
          </For>
        </Show>
        <RelaySettingsSection />
      </>
    );
  }

  // Architecture §17.3 — dedicated Mobile tab. Surfaces the three-tier
  // notification escalation explanation + push relay registration.
  // Keeping this separate from Privacy makes the "I'm setting up a
  // mobile workflow" intent obvious.
  function renderMobile() {
    return (
      <>
        <PushRelaySettingsSection />
      </>
    );
  }

  function renderDevices() {
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
  }

  function renderAbout() {
    return (
      <>
        <div class="settings-section-title">Rekindle</div>
        <div class="settings-about-text">
          A faithful recreation of the classic Xfire gaming chat client,
          rebuilt with modern technology for the decentralized era.
        </div>
        <div class="settings-about-row">
          <span class="form-field-label">Version</span>
          <span class="settings-about-value">0.1.0-dev</span>
        </div>
        <div class="settings-about-row">
          <span class="form-field-label">Stack</span>
          <span class="settings-about-value">Tauri 2 + SolidJS + Veilid</span>
        </div>
        <div class="settings-about-row">
          <span class="form-field-label">License</span>
          <span class="settings-about-value">MIT</span>
        </div>
        <div class="settings-section-title">Updates</div>
        <div class="update-check-row">
          <button
            class="form-btn-secondary"
            onClick={handleCheckUpdates}
            disabled={checkingUpdates()}
          >
            {checkingUpdates() ? "Checking..." : "Check for Updates"}
          </button>
          <Show when={updateResult()}>
            {(result) => (
              <span class={result().includes("available") ? "update-status-available" : "update-status"}>
                {result()}
              </span>
            )}
          </Show>
        </div>
      </>
    );
  }

  return (
    <div class="app-frame">
      {/* Architecture §32 a11y — keyboard skip link past tab rail. */}
      <a href="#main-content" class="skip-link">Skip to settings content</a>
      <Titlebar title="Settings" />
      <div class="form-tabs">
        <For each={TAB_LABELS}>
          {(tab) => (
            <button
              class={`form-tab ${activeTab() === tab.id ? "form-tab-active" : ""}`}
              onClick={() => setActiveTab(tab.id)}
            >
              {tab.label}
            </button>
          )}
        </For>
      </div>
      <div class="settings-content" id="main-content" tabindex="-1">
        <Show when={activeTab() === "profile"}>{renderProfile()}</Show>
        <Show when={activeTab() === "application"}>{renderApplication()}</Show>
        <Show when={activeTab() === "notifications"}>{renderNotifications()}</Show>
        <Show when={activeTab() === "audio"}>{renderAudio()}</Show>
        <Show when={activeTab() === "video"}>{renderVideo()}</Show>
        <Show when={activeTab() === "privacy"}>{renderPrivacy()}</Show>
        <Show when={activeTab() === "devices"}>{renderDevices()}</Show>
        <Show when={activeTab() === "mobile"}>{renderMobile()}</Show>
        <Show when={activeTab() === "about"}>{renderAbout()}</Show>
      </div>
    </div>
  );
};

export default SettingsWindow;
