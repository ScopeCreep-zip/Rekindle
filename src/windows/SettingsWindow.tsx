import { Component, createSignal, For, Show, onMount } from "solid-js";
import Titlebar from "../components/titlebar/Titlebar";
import Avatar from "../components/common/Avatar";
import { settingsState } from "../stores/settings.store";
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

type SettingsTab = "general" | "notifications" | "audio" | "privacy" | "about";

const TAB_LABELS: { id: SettingsTab; label: string }[] = [
  { id: "general", label: "General" },
  { id: "notifications", label: "Notifications" },
  { id: "audio", label: "Audio" },
  { id: "privacy", label: "Privacy" },
  { id: "about", label: "About" },
];

const SettingsWindow: Component = () => {
  const [activeTab, setActiveTab] = createSignal<SettingsTab>("general");
  const [nameInput, setNameInput] = createSignal("");
  const [statusMsgInput, setStatusMsgInput] = createSignal("");
  const [checkingUpdates, setCheckingUpdates] = createSignal(false);
  const [updateResult, setUpdateResult] = createSignal<string | null>(null);

  onMount(() => {
    hydrateState().then(() => {
      setNameInput(authState.displayName ?? "");
    });
    handleLoadSettings();
  });

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

  function renderGeneral() {
    return (
      <>
        <div class="settings-section-title">Profile</div>
        <div class="avatar-upload-section">
          <Avatar displayName={authState.displayName ?? "?"} size={64} avatarUrl={authState.avatarUrl ?? undefined} />
          <button class="avatar-upload-btn" onClick={handleAvatarUpload}>
            Change Avatar
          </button>
          <span class="avatar-upload-hint">PNG, JPEG, or GIF (max 256KB)</span>
        </div>
        <div class="settings-field">
          <label class="settings-field-label">Display Name</label>
          <div class="settings-field-row">
            <input
              class="settings-input"
              type="text"
              value={nameInput()}
              onInput={(e: InputEvent) => setNameInput((e.target as HTMLInputElement).value)}
              onKeyDown={(e: KeyboardEvent) => { if (e.key === "Enter") handleSaveName(); }}
            />
            <button class="settings-save-btn" onClick={handleSaveName}>Save</button>
          </div>
        </div>
        <div class="settings-field">
          <label class="settings-field-label">Status Message</label>
          <div class="settings-field-row">
            <input
              class="settings-input"
              type="text"
              placeholder="What's on your mind?"
              value={statusMsgInput()}
              onInput={(e: InputEvent) => setStatusMsgInput((e.target as HTMLInputElement).value)}
              onKeyDown={(e: KeyboardEvent) => { if (e.key === "Enter") handleSaveStatusMessage(); }}
            />
            <button class="settings-save-btn" onClick={handleSaveStatusMessage}>Save</button>
          </div>
        </div>
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
        <label class="settings-option">
          <input
            type="checkbox"
            checked={settingsState.showGameActivity}
            onChange={() => handleToggle("showGameActivity")}
          />
          <span class="buddy-name">Show Game Activity</span>
        </label>
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
      </>
    );
  }

  function renderAudio() {
    return (
      <>
        <div class="settings-section-title">Audio Devices</div>
        <div class="settings-field">
          <label class="settings-field-label">Input Device</label>
          <select class="settings-select" disabled>
            <option>Default Microphone</option>
          </select>
        </div>
        <div class="settings-field">
          <label class="settings-field-label">Output Device</label>
          <select class="settings-select" disabled>
            <option>Default Speakers</option>
          </select>
        </div>
        <div class="settings-hint">Audio device selection requires voice to be connected.</div>
      </>
    );
  }

  function renderPrivacy() {
    return (
      <>
        <div class="settings-section-title">Privacy</div>
        <div class="settings-field">
          <label class="settings-field-label">Public Key</label>
          <div class="profile-key-display">{authState.publicKey ?? "Not logged in"}</div>
        </div>
        <div class="settings-section-title">Identity</div>
        <div class="settings-field-row">
          <button class="settings-action-btn" disabled>Export Identity</button>
          <button class="settings-action-btn" disabled>Import Identity</button>
        </div>
        <div class="settings-hint">Identity export/import requires Stronghold integration.</div>
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
          <span class="settings-field-label">Version</span>
          <span class="settings-about-value">0.1.0-dev</span>
        </div>
        <div class="settings-about-row">
          <span class="settings-field-label">Stack</span>
          <span class="settings-about-value">Tauri 2 + SolidJS + Veilid</span>
        </div>
        <div class="settings-about-row">
          <span class="settings-field-label">License</span>
          <span class="settings-about-value">MIT</span>
        </div>
        <div class="settings-section-title">Updates</div>
        <div class="update-check-row">
          <button
            class="settings-action-btn"
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
      <Titlebar title="Settings" />
      <div class="settings-tabs">
        <For each={TAB_LABELS}>
          {(tab) => (
            <button
              class={`settings-tab ${activeTab() === tab.id ? "settings-tab-active" : ""}`}
              onClick={() => setActiveTab(tab.id)}
            >
              {tab.label}
            </button>
          )}
        </For>
      </div>
      <div class="settings-content">
        <Show when={activeTab() === "general"}>{renderGeneral()}</Show>
        <Show when={activeTab() === "notifications"}>{renderNotifications()}</Show>
        <Show when={activeTab() === "audio"}>{renderAudio()}</Show>
        <Show when={activeTab() === "privacy"}>{renderPrivacy()}</Show>
        <Show when={activeTab() === "about"}>{renderAbout()}</Show>
      </div>
    </div>
  );
};

export default SettingsWindow;
