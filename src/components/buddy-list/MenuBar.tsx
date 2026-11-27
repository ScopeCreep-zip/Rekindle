import type { Component } from "solid-js";
import { Menubar } from "@kobalte/core/menubar";
import { handleOpenDocs, handleQuit, handleReportBug, openSettingsTab } from "../../actions/app_menu.actions";
import { setBuddyListUI } from "../../stores/buddylist-ui.store";

/**
 * Plan GC4 (live, once): the Rekindle menu's dropdown did not show right
 * after a call started. From code, a starting call changes the buddy list
 * three ways: the call window is revealed with focus (`windows/reveal.rs`
 * `set_focus`), so this window goes inactive and, on macOS, its next click
 * only activates it (no window sets `accept_first_mouse`); the outgoing-call
 * panel (`.outgoing-call-panel`, fixed top-right, z-index 75) sits over the
 * menu bar's right side; and a modal Dialog (Start Group Call, incoming
 * call) leaves `pointer-events: none` on the body while it unmounts. This
 * identifier-free log tells them apart live: a click whose `pointerdown`
 * never logs never reached the page (first mouse / covered); a
 * `pointerdown` with no `open` line, or `bodyPointerEvents: "none"`, is a
 * leftover modal; an `open` line with no dropdown is a render fault.
 */
function menuDebug(menu: string, what: string): void {
  console.debug("[menubar]", menu, what, {
    windowFocused: document.hasFocus(),
    visibility: document.visibilityState,
    bodyPointerEvents: getComputedStyle(document.body).pointerEvents,
    dialogsOpen: document.querySelectorAll("[role=dialog],[role=alertdialog]").length,
  });
}

const MenuBar: Component = () => {
  return (
    <Menubar class="menu-bar">
      <Menubar.Menu onOpenChange={(o) => menuDebug("Rekindle", o ? "open" : "close")}>
        <Menubar.Trigger class="menu-bar-item" onPointerDown={() => menuDebug("Rekindle", "pointerdown")}>
          Rekindle
        </Menubar.Trigger>
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
      <Menubar.Menu onOpenChange={(o) => menuDebug("Calls", o ? "open" : "close")}>
        <Menubar.Trigger class="menu-bar-item" onPointerDown={() => menuDebug("Calls", "pointerdown")}>
          Calls
        </Menubar.Trigger>
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

      <Menubar.Menu onOpenChange={(o) => menuDebug("Help", o ? "open" : "close")}>
        <Menubar.Trigger class="menu-bar-item" onPointerDown={() => menuDebug("Help", "pointerdown")}>
          Help
        </Menubar.Trigger>
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
