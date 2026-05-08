import { Component } from "solid-js";
import { Menubar } from "@kobalte/core/menubar";
import { openUrl } from "@tauri-apps/plugin-opener";
import { commands } from "../../ipc/commands";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { setBuddyListUI } from "../../stores/buddylist-ui.store";

const REPO_URL = "https://github.com/ScopeCreep-zip/Rekindle";

function openSettingsTab(tab: string): void {
  commands.openSettingsWindow(tab);
}

function handleQuit(): void {
  getCurrentWindow().close();
}

function handleOpenDocs(): void {
  openUrl(`${REPO_URL}/tree/main/docs`);
}

function handleReportBug(): void {
  openUrl(`${REPO_URL}/issues/new`);
}

const MenuBar: Component = () => {
  return (
    <Menubar class="menu-bar">
      <Menubar.Menu>
        <Menubar.Trigger class="menu-bar-item">Rekindle</Menubar.Trigger>
        <Menubar.Portal>
          <Menubar.Content class="menu-dropdown">
            <Menubar.Item class="menu-dropdown-item" onSelect={() => openSettingsTab("profile")}>
              Profile
            </Menubar.Item>
            <Menubar.Item class="menu-dropdown-item" onSelect={() => openSettingsTab("application")}>
              Application
            </Menubar.Item>
            <Menubar.Item class="menu-dropdown-item" onSelect={() => openSettingsTab("notifications")}>
              Notifications
            </Menubar.Item>
            <Menubar.Item class="menu-dropdown-item" onSelect={() => openSettingsTab("audio")}>
              Audio
            </Menubar.Item>
            <Menubar.Item class="menu-dropdown-item" onSelect={() => openSettingsTab("privacy")}>
              Privacy
            </Menubar.Item>
            <Menubar.Item class="menu-dropdown-item" onSelect={() => openSettingsTab("about")}>
              About
            </Menubar.Item>
            <Menubar.Separator class="menu-dropdown-separator" />
            <Menubar.Item
              class="menu-dropdown-item menu-dropdown-item-danger"
              onSelect={handleQuit}
            >
              Quit
            </Menubar.Item>
          </Menubar.Content>
        </Menubar.Portal>
      </Menubar.Menu>

      {/* Wave 12 W12.10 — group call entry point. */}
      <Menubar.Menu>
        <Menubar.Trigger class="menu-bar-item">Calls</Menubar.Trigger>
        <Menubar.Portal>
          <Menubar.Content class="menu-dropdown">
            <Menubar.Item
              class="menu-dropdown-item"
              onSelect={() => setBuddyListUI("showStartGroupCall", true)}
            >
              Start Group Call…
            </Menubar.Item>
          </Menubar.Content>
        </Menubar.Portal>
      </Menubar.Menu>

      <Menubar.Menu>
        <Menubar.Trigger class="menu-bar-item">Help</Menubar.Trigger>
        <Menubar.Portal>
          <Menubar.Content class="menu-dropdown">
            <Menubar.Item class="menu-dropdown-item" onSelect={handleOpenDocs}>
              Documentation
            </Menubar.Item>
            <Menubar.Item class="menu-dropdown-item" onSelect={handleReportBug}>
              Report Bug
            </Menubar.Item>
          </Menubar.Content>
        </Menubar.Portal>
      </Menubar.Menu>
    </Menubar>
  );
};

export default MenuBar;
