import { Component, Show, onCleanup } from "solid-js";
import { openUrl } from "@tauri-apps/plugin-opener";
import { buddyListUI, setBuddyListUI } from "../../stores/buddylist-ui.store";
import { commands } from "../../ipc/commands";
import { getCurrentWindow } from "@tauri-apps/api/window";

const REPO_URL = "https://github.com/ScopeCreep-zip/Rekindle";

function toggleMenu(name: string): void {
  setBuddyListUI("menuOpen", buddyListUI.menuOpen === name ? null : name);
}

function closeMenus(): void {
  setBuddyListUI("menuOpen", null);
}

function openSettingsTab(tab: string): void {
  closeMenus();
  commands.openSettingsWindow(tab);
}

function handleQuit(): void {
  closeMenus();
  getCurrentWindow().close();
}

function handleOpenDocs(): void {
  closeMenus();
  openUrl(`${REPO_URL}/tree/main/docs`);
}

function handleReportBug(): void {
  closeMenus();
  openUrl(`${REPO_URL}/issues/new`);
}

const MenuBar: Component = () => {
  function handleClickOutside(e: MouseEvent): void {
    const target = e.target as HTMLElement;
    if (!target.closest(".menu-bar")) {
      closeMenus();
    }
  }

  document.addEventListener("click", handleClickOutside);
  onCleanup(() => document.removeEventListener("click", handleClickOutside));

  return (
    <div class="menu-bar">
      <div class="menu-bar-item-wrapper">
        <button
          class={`menu-bar-item ${buddyListUI.menuOpen === "rekindle" ? "menu-bar-item-active" : ""}`}
          onClick={() => toggleMenu("rekindle")}
        >
          Rekindle
        </button>
        <Show when={buddyListUI.menuOpen === "rekindle"}>
          <div class="menu-dropdown">
            <div class="menu-dropdown-item" onClick={() => openSettingsTab("profile")}>Profile</div>
            <div class="menu-dropdown-item" onClick={() => openSettingsTab("application")}>Application</div>
            <div class="menu-dropdown-item" onClick={() => openSettingsTab("notifications")}>Notifications</div>
            <div class="menu-dropdown-item" onClick={() => openSettingsTab("audio")}>Audio</div>
            <div class="menu-dropdown-item" onClick={() => openSettingsTab("privacy")}>Privacy</div>
            <div class="menu-dropdown-item" onClick={() => openSettingsTab("about")}>About</div>
            <div class="menu-dropdown-separator" />
            <div class="menu-dropdown-item menu-dropdown-item-danger" onClick={handleQuit}>Quit</div>
          </div>
        </Show>
      </div>
      <div class="menu-bar-item-wrapper">
        <button
          class={`menu-bar-item ${buddyListUI.menuOpen === "help" ? "menu-bar-item-active" : ""}`}
          onClick={() => toggleMenu("help")}
        >
          Help
        </button>
        <Show when={buddyListUI.menuOpen === "help"}>
          <div class="menu-dropdown">
            <div class="menu-dropdown-item" onClick={handleOpenDocs}>Documentation</div>
            <div class="menu-dropdown-item" onClick={handleReportBug}>Report Bug</div>
          </div>
        </Show>
      </div>
    </div>
  );
};

export default MenuBar;
