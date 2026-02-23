import { Component, createSignal, createEffect, For, Show, onMount, onCleanup } from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import Titlebar from "../components/titlebar/Titlebar";
import Avatar from "../components/common/Avatar";
import FormField from "../components/common/FormField";
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

function getInitialTab(): SettingsTab {
  const params = new URLSearchParams(window.location.search);
  const tab = params.get("tab");
  const valid: SettingsTab[] = ["profile", "application", "notifications", "audio", "privacy", "about"];
  if (tab && valid.includes(tab as SettingsTab)) {
    return tab as SettingsTab;
  }
  return "profile";
}

type SettingsTab = "profile" | "application" | "notifications" | "audio" | "privacy" | "about";

const TAB_LABELS: { id: SettingsTab; label: string }[] = [
  { id: "profile", label: "Profile" },
  { id: "application", label: "Application" },
  { id: "notifications", label: "Notifications" },
  { id: "audio", label: "Audio" },
  { id: "privacy", label: "Privacy" },
  { id: "about", label: "About" },
];

const SettingsWindow: Component = () => {
  const [activeTab, setActiveTab] = createSignal<SettingsTab>(getInitialTab());
  const [nameInput, setNameInput] = createSignal("");
  const [statusMsgInput, setStatusMsgInput] = createSignal("");
  const [checkingUpdates, setCheckingUpdates] = createSignal(false);
  const [updateResult, setUpdateResult] = createSignal<string | null>(null);
  const [blockedUsers, setBlockedUsers] = createSignal<{ publicKey: string; displayName: string; blockedAt: number }[]>([]);

  let unlistenSwitchTab: Promise<UnlistenFn> | undefined;

  onMount(() => {
    hydrateState().then(() => {
      setNameInput(authState.displayName ?? "");
    });
    handleLoadSettings();

    unlistenSwitchTab = listen<string>("settings-switch-tab", (event) => {
      const valid: SettingsTab[] = ["profile", "application", "notifications", "audio", "privacy", "about"];
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
            <button class="form-btn-save" onClick={handleSaveName}>Save</button>
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
            <button class="form-btn-save" onClick={handleSaveStatusMessage}>Save</button>
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
      </>
    );
  }

  function renderAudio() {
    return (
      <>
        <div class="settings-section-title">Audio Devices</div>
        <FormField label="Input Device">
          <select class="form-select" disabled>
            <option>Default Microphone</option>
          </select>
        </FormField>
        <FormField label="Output Device">
          <select class="form-select" disabled>
            <option>Default Speakers</option>
          </select>
        </FormField>
        <div class="settings-hint">Audio device selection requires voice to be connected.</div>
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
      <div class="settings-content">
        <Show when={activeTab() === "profile"}>{renderProfile()}</Show>
        <Show when={activeTab() === "application"}>{renderApplication()}</Show>
        <Show when={activeTab() === "notifications"}>{renderNotifications()}</Show>
        <Show when={activeTab() === "audio"}>{renderAudio()}</Show>
        <Show when={activeTab() === "privacy"}>{renderPrivacy()}</Show>
        <Show when={activeTab() === "about"}>{renderAbout()}</Show>
      </div>
    </div>
  );
};

export default SettingsWindow;
