import { type Component, For, type JSX, Show, createContext, useContext } from "solid-js";
import { Menubar } from "@kobalte/core/menubar";
import { ContextMenu } from "@kobalte/core/context-menu";
import type { EngineMenu, MenuItem } from "../../ipc/commands/dto_skin_tree";

/// The accessible name of a menu title the Host serves no label for: its
/// art already carries the right word (Tools, Help), or it is art alone (a
/// logo opening the window menu), which has no name otherwise (axe
/// button-name). A served label (plan SK4b) names the title instead.
const ART_NAME: Record<EngineMenu, string> = {
  file: "Rekindle",
  tool: "Tools",
  help: "Help",
  status: "Status",
  sys: "Window menu",
  cwActions: "Actions",
  cwHelp: "Help",
  gcSystem: "Window menu",
  gcActions: "Actions",
  gcHelp: "Help",
};

/** Whether a menu sits inside the pack's menu bar (one Kobalte `Menubar`,
 *  so hover moves between open menus). */
export const InBar = createContext(false);

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
  /** The served label's words (plan SK4b), drawn by the node's NodeLabelView. */
  label?: string;
  items: MenuItem[];
  run: (item: MenuItem) => void;
  children: JSX.Element;
}

const Menu: Component<MenuProps> = (props) => (
  <Menubar.Menu>
    <Menubar.Trigger class="skin-menu" {...props.attrs} aria-label={props.label ?? ART_NAME[props.menu]}>
      {props.children}
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
