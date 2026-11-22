import { Component, For, JSX, Show, createContext, useContext } from "solid-js";
import { Menubar } from "@kobalte/core/menubar";
import type { EngineMenu } from "../../ipc/commands/dto_skin_tree";

/** The engine menus a pack's menu titles open here (`Menu<Name>` triggers). */
export type MenuKey = Extract<EngineMenu, "file" | "tool" | "help">;
import { handleLogout } from "../../actions/auth.actions";
import { handleToggleAddFriend } from "../../actions/buddy.actions";
import { setBuddyListUI } from "../../stores/buddylist-ui.store";
import { setShortcutsHelpOpen } from "../../stores/shortcuts_help.store";
import {
  handleOpenDocs,
  handleQuit,
  handleReportBug,
  openSettingsTab,
} from "../buddy-list/MenuBar";
import {
  handleToggleCreateCommunity,
  handleToggleJoinCommunity,
  handleToggleNewChat,
} from "../buddy-list/BottomActionBar";

interface Item {
  label: string;
  run: () => void;
  /** A separator before this item. */
  apart?: boolean;
  danger?: boolean;
}

/// The first menu reads "Rekindle" (plan SK2 lead decision 7: the Xfire
/// name is a trademark); the others keep Xfire's names (lang 229/231).
const TITLE: Record<MenuKey, string> = { file: "Rekindle", tool: "Tools", help: "Help" };

/// N2 (design §3): our menu items regrouped under the pack's menus. The
/// first menu holds the account and contact actions and Log Off/Exit
/// (Xfire's own placement; Create/Join Community per design §4), Tools
/// the options, skins, group call and shortcuts, Help the docs.
const ITEMS: Record<MenuKey, Item[]> = {
  file: [
    { label: "Add Friend…", run: handleToggleAddFriend },
    { label: "New Chat…", run: handleToggleNewChat },
    { label: "New Group…", run: () => setBuddyListUI("showNewGroup", true) },
    { label: "Create Community…", run: handleToggleCreateCommunity, apart: true },
    { label: "Join Community…", run: handleToggleJoinCommunity },
    { label: "Profile", run: () => openSettingsTab("profile"), apart: true },
    { label: "Privacy", run: () => openSettingsTab("privacy") },
    { label: "About", run: () => openSettingsTab("about") },
    { label: "Log Off", run: () => void handleLogout(), apart: true },
    { label: "Exit", run: handleQuit, danger: true },
  ],
  tool: [
    { label: "Options", run: () => openSettingsTab("application") },
    { label: "Notifications", run: () => openSettingsTab("notifications") },
    { label: "Audio", run: () => openSettingsTab("audio") },
    { label: "Skins", run: () => openSettingsTab("appearance") },
    { label: "Start Group Call…", run: () => setBuddyListUI("showStartGroupCall", true), apart: true },
    { label: "Keyboard Shortcuts", run: () => setShortcutsHelpOpen(true) },
  ],
  help: [
    { label: "Documentation", run: handleOpenDocs },
    { label: "Report Bug", run: handleReportBug },
  ],
};

/** Whether a menu sits inside the pack's menu bar (one Kobalte `Menubar`,
 *  so hover moves between open menus). */
const InBar = createContext(false);

/** The pack's `MenuBar` node: the menus' container. */
export const PackMenuBar: Component<{ id: string; children: JSX.Element }> = (props) => (
  <Menubar class="skin-menubar" data-skin-node={props.id}>
    <InBar.Provider value={true}>{props.children}</InBar.Provider>
  </Menubar>
);

const MenuItems: Component<{ menu: MenuKey }> = (props) => (
  <Menubar.Portal>
    <Menubar.Content class="menu-dropdown">
      <For each={ITEMS[props.menu]}>
        {(item) => (
          <>
            <Show when={item.apart}>
              <Menubar.Separator class="menu-dropdown-separator" />
            </Show>
            <Menubar.Item
              class={item.danger ? "menu-dropdown-item menu-dropdown-item-danger" : "menu-dropdown-item"}
              onSelect={item.run}
            >
              {item.label}
            </Menubar.Item>
          </>
        )}
      </For>
    </Menubar.Content>
  </Menubar.Portal>
);

const Menu: Component<{ id: string; menu: MenuKey; children: JSX.Element }> = (props) => (
  <Menubar.Menu>
    <Menubar.Trigger class="skin-menu" data-skin-node={props.id}>
      {props.children}
      <span class="skin-menu-label">{TITLE[props.menu]}</span>
    </Menubar.Trigger>
    <MenuItems menu={props.menu} />
  </Menubar.Menu>
);

/** One menu title at the pack's place; a menu the pack lays outside a menu
 *  bar is its own one-menu bar. */
export const PackMenu: Component<{ id: string; menu: MenuKey; children: JSX.Element }> = (props) => (
  <Show
    when={useContext(InBar)}
    fallback={
      <Menubar class="skin-menubar skin-menubar-lone">
        <Menu id={props.id} menu={props.menu}>
          {props.children}
        </Menu>
      </Menubar>
    }
  >
    <Menu id={props.id} menu={props.menu}>
      {props.children}
    </Menu>
  </Show>
);
