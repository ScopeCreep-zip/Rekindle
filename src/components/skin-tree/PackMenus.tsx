import { Component, For, JSX, Show, createContext, useContext } from "solid-js";
import { Menubar } from "@kobalte/core/menubar";
import { ContextMenu } from "@kobalte/core/context-menu";
import type { EngineMenu, MenuItem } from "../../ipc/commands/dto_skin_tree";

/// The menu title's text: the first menu reads "Rekindle" (plan SK2 lead
/// decision 7: the Xfire name is a trademark), the others keep Xfire's
/// names (lang 229/231).
const TITLE: Partial<Record<EngineMenu, string>> = { file: "Rekindle", tool: "Tools", help: "Help", status: "Status" };

/** Whether a menu sits inside the pack's menu bar (one Kobalte `Menubar`,
 *  so hover moves between open menus). */
const InBar = createContext(false);

/** The pack's `MenuBar` node: the menus' container. */
export const PackMenuBar: Component<{ attrs: Record<string, unknown>; children: JSX.Element }> = (props) => (
  <Menubar class="skin-menubar" {...props.attrs}>
    <InBar.Provider value={true}>{props.children}</InBar.Provider>
  </Menubar>
);

/** The served items of one engine menu (the Host's `MenuRegistry`), each
 *  run through the window's dispatcher. */
const BarItems: Component<{ items: MenuItem[]; run: (item: MenuItem) => void }> = (props) => (
  <Menubar.Portal>
    <Menubar.Content class="menu-dropdown">
      <For each={props.items}>
        {(item) => (
          <>
            <Show when={item.apart}>
              <Menubar.Separator class="menu-dropdown-separator" />
            </Show>
            <Menubar.Item
              class={item.danger ? "menu-dropdown-item menu-dropdown-item-danger" : "menu-dropdown-item"}
              onSelect={() => props.run(item)}
            >
              {item.label}
            </Menubar.Item>
          </>
        )}
      </For>
    </Menubar.Content>
  </Menubar.Portal>
);

interface MenuProps {
  attrs: Record<string, unknown>;
  menu: EngineMenu;
  items: MenuItem[];
  run: (item: MenuItem) => void;
  children: JSX.Element;
}

const Menu: Component<MenuProps> = (props) => (
  <Menubar.Menu>
    <Menubar.Trigger class="skin-menu" {...props.attrs}>
      {props.children}
      <Show when={TITLE[props.menu]}>{(t) => <span class="skin-menu-label">{t()}</span>}</Show>
    </Menubar.Trigger>
    <BarItems items={props.items} run={props.run} />
  </Menubar.Menu>
);

/** A menu title the pack opens with a press (`Menu<Name>`); one laid
 *  outside a menu bar is its own one-menu bar. */
export const PackMenu: Component<MenuProps> = (props) => (
  <Show
    when={useContext(InBar)}
    fallback={
      <Menubar class="skin-menubar skin-menubar-lone">
        <Menu {...props} />
      </Menubar>
    }
  >
    <Menu {...props} />
  </Show>
);

/** A node the pack opens a menu on with a right-click (`ContextMenu<Name>`,
 *  XGlow's right-click `MenuTool`): the node itself is the trigger. */
export const PackContextMenu: Component<Omit<MenuProps, "menu">> = (props) => (
  <ContextMenu>
    <ContextMenu.Trigger as="div" {...props.attrs}>
      {props.children}
    </ContextMenu.Trigger>
    <ContextMenu.Portal>
      <ContextMenu.Content class="context-menu">
        <For each={props.items}>
          {(item) => (
            <>
              <Show when={item.apart}>
                <ContextMenu.Separator class="menu-dropdown-separator" />
              </Show>
              <ContextMenu.Item
                class={item.danger ? "context-menu-item menu-dropdown-item-danger" : "context-menu-item"}
                onSelect={() => props.run(item)}
              >
                {item.label}
              </ContextMenu.Item>
            </>
          )}
        </For>
      </ContextMenu.Content>
    </ContextMenu.Portal>
  </ContextMenu>
);
