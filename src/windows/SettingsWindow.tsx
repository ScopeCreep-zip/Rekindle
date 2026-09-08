import { Component, createSignal, For, Show, onMount, onCleanup } from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import Titlebar from "../components/titlebar/Titlebar";
import PushRelaySettingsSection from "../components/settings/PushRelaySettingsSection";
import { handleLoadSettings } from "../actions/settings.actions";
import { hydrateState } from "../stores/hydrate";
import ProfileTab from "./settings/ProfileTab";
import ApplicationTab from "./settings/ApplicationTab";
import NotificationsTab from "./settings/NotificationsTab";
import AudioTab from "./settings/AudioTab";
import VideoTab from "./settings/VideoTab";
import PrivacyTab from "./settings/PrivacyTab";
import DevicesTab from "./settings/DevicesTab";
import AboutTab from "./settings/AboutTab";

type SettingsTab = "profile" | "application" | "notifications" | "audio" | "video" | "privacy" | "devices" | "mobile" | "about";

const VALID_TABS: SettingsTab[] = ["profile", "application", "notifications", "audio", "video", "privacy", "devices", "mobile", "about"];

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

function getInitialTab(): SettingsTab {
  const params = new URLSearchParams(window.location.search);
  const tab = params.get("tab");
  if (tab && VALID_TABS.includes(tab as SettingsTab)) {
    return tab as SettingsTab;
  }
  return "profile";
}

const SettingsWindow: Component = () => {
  const [activeTab, setActiveTab] = createSignal<SettingsTab>(getInitialTab());

  let unlistenSwitchTab: Promise<UnlistenFn> | undefined;

  onMount(() => {
    // Hydrate global stores read by every tab (identity, preferences).
    void hydrateState();
    handleLoadSettings();

    unlistenSwitchTab = listen<string>("settings-switch-tab", (event) => {
      if (VALID_TABS.includes(event.payload as SettingsTab)) {
        setActiveTab(event.payload as SettingsTab);
      }
    });
  });

  onCleanup(() => {
    unlistenSwitchTab?.then((unlisten) => unlisten());
  });

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
        <Show when={activeTab() === "profile"}><ProfileTab /></Show>
        <Show when={activeTab() === "application"}><ApplicationTab /></Show>
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
