import { Component, createSignal, For, Show, Suspense, onMount, onCleanup } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { startEventStream, subscribeSettingsTab } from "../ipc/channels";
import Titlebar from "../components/titlebar/Titlebar";
import PushRelaySettingsSection from "../components/settings/PushRelaySettingsSection";
import { handleLoadSettings } from "../actions/settings.actions";
import { hydrateState } from "../stores/hydrate";
import type { SettingsTab } from "../ipc/commands/types";
import ProfileTab from "./settings/ProfileTab";
import ApplicationTab from "./settings/ApplicationTab";
import AppearanceTab from "./settings/AppearanceTab";
import NotificationsTab from "./settings/NotificationsTab";
import AudioTab from "./settings/AudioTab";
import VideoTab from "./settings/VideoTab";
import PrivacyTab from "./settings/PrivacyTab";
import DevicesTab from "./settings/DevicesTab";
import AboutTab from "./settings/AboutTab";
import { rovingList } from "../utils/roving_list";
import { setWindowKind } from "../utils/window_kind";


const TAB_LABELS: { id: SettingsTab; label: string }[] = [
  { id: "profile", label: "Profile" },
  { id: "application", label: "Application" },
  { id: "appearance", label: "Appearance" },
  { id: "notifications", label: "Notifications" },
  { id: "audio", label: "Audio" },
  { id: "video", label: "Video" },
  { id: "privacy", label: "Privacy" },
  { id: "devices", label: "Devices" },
  { id: "mobile", label: "Mobile" },
  { id: "about", label: "About" },
];

function getInitialTab(): SettingsTab {
  const requested = new URLSearchParams(window.location.search).get("tab");
  return TAB_LABELS.find((tab) => tab.id === requested)?.id ?? "profile";
}

const SettingsWindow: Component = () => {
  const [activeTab, setActiveTab] = createSignal<SettingsTab>(getInitialTab());

  let unlistenSwitchTab: Promise<UnlistenFn> | undefined;

  onMount(() => {
    // Hydrate global stores read by every tab (identity, preferences).
    void hydrateState();
    handleLoadSettings();

    unlistenSwitchTab = subscribeSettingsTab(setActiveTab);
    void startEventStream(() => void hydrateState());
  });

  onCleanup(() => {
    unlistenSwitchTab?.then((unlisten) => unlisten());
  });

  setWindowKind("settings");
  return (
    <div class="app-frame">
      {/* Architecture §32 a11y — keyboard skip link past tab rail. */}
      <a href="#main-content" class="skip-link">Skip to settings content</a>
      <Titlebar title="Settings" />
      {/* Finding V4: `activeTab` is the one selection; the rail exposes it as
          a WAI-ARIA tablist (WCAG 4.1.2) and the active look keys off
          `aria-selected`, so the painted tab, the read one and the content
          cannot disagree. */}
      <div
        ref={rovingList('[role="tab"]', { orientation: "horizontal", preferSelected: true })}
        class="form-tabs"
        role="tablist"
        aria-label="Settings sections"
      >
        <For each={TAB_LABELS}>
          {(tab) => (
            <button
              type="button"
              role="tab"
              id={`settings-tab-${tab.id}`}
              aria-selected={activeTab() === tab.id}
              aria-controls="main-content"
              class={`form-tab ${activeTab() === tab.id ? "form-tab-active" : ""}`}
              onFocus={() => setActiveTab(tab.id)}
              onClick={() => setActiveTab(tab.id)}
            >
              {tab.label}
            </button>
          )}
        </For>
      </div>
      <div class="settings-content" id="main-content" tabindex="0" role="tabpanel" aria-labelledby={`settings-tab-${activeTab()}`}>
        <Show when={activeTab() === "profile"}>
          {/* Keep the rail focused while the Profile resources load. */}
          <Suspense><ProfileTab /></Suspense>
        </Show>
        <Show when={activeTab() === "application"}><ApplicationTab /></Show>
        <Show when={activeTab() === "appearance"}><AppearanceTab /></Show>
        <Show when={activeTab() === "notifications"}><NotificationsTab /></Show>
        <Show when={activeTab() === "audio"}><AudioTab /></Show>
        <Show when={activeTab() === "video"}><VideoTab /></Show>
        <Show when={activeTab() === "privacy"}><PrivacyTab /></Show>
        <Show when={activeTab() === "devices"}><DevicesTab /></Show>
        <Show when={activeTab() === "mobile"}><PushRelaySettingsSection /></Show>
        <Show when={activeTab() === "about"}><AboutTab /></Show>
      </div>
    </div>
  );
};

export default SettingsWindow;
