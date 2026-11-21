import { Component, For, JSX, Match, Switch, createContext, useContext } from "solid-js";
import type { ShellNode, SkinShell, ViewKey } from "../../ipc/commands/dto_skin_tree";
import { handleHide, handleMaximize, handleMinimize } from "../../actions/titlebar.actions";
import NameField from "./NameField";
import { PackMenu, PackMenuBar } from "./PackMenus";
import { StatusList, StatusText } from "./StatusField";
import AdArea from "./AdArea";
import ResizeHandle from "./ResizeHandle";

/// The view the tree shows. Plan SK2-S2 binds the Friends view only; the
/// tab strip that switches views is slice S3.
const SHOWN_VIEW: ViewKey = "friends";

/** What every node of one window's tree reads: the content its views hold. */
const TreeCtx = createContext<{ views: () => JSX.Element; hasFriendsView: boolean }>();

function useTree(): { views: () => JSX.Element; hasFriendsView: boolean } {
  const ctx = useContext(TreeCtx);
  if (!ctx) throw new Error("a skin node outside SkinTreeWindow");
  return ctx;
}

function hasFriendsView(nodes: ShellNode[]): boolean {
  return nodes.some(
    (n) => (n.role.kind === "view" && n.role.key === "friends") || hasFriendsView(n.children ?? []),
  );
}

const Children: Component<{ node: ShellNode }> = (props) => (
  <For each={props.node.children ?? []}>{(c) => <SkinNodeView node={c} />}</For>
);

/** A node with no behaviour of its own yet (later slices bind it): present
 *  and sized by its rule, holding its children. */
const Plain: Component<{ node: ShellNode; drag?: boolean; hidden?: boolean }> = (props) => (
  <div data-skin-node={props.node.id} data-tauri-drag-region={props.drag ? true : undefined} hidden={props.hidden}>
    <Children node={props.node} />
  </div>
);

const TitleButton: Component<{ node: ShellNode; label: string; onClick: () => void }> = (props) => (
  <button type="button" class="skin-tree-button" data-skin-node={props.node.id} aria-label={props.label} onClick={() => props.onClick()}>
    <Children node={props.node} />
  </button>
);

/** One node, bound by its role (design §3). */
const SkinNodeView: Component<{ node: ShellNode }> = (props) => {
  const tree = useTree();
  const n = (): ShellNode => props.node;
  const role = (): ShellNode["role"] => props.node.role;
  const offView = (): boolean => n().showsWith !== undefined && n().showsWith !== SHOWN_VIEW;
  const viewKey = (): ViewKey | undefined => {
    const r = role();
    return r.kind === "view" || r.kind === "buttonRow" ? r.key : undefined;
  };
  return (
    <Switch fallback={<Plain node={n()} hidden={offView()} />}>
      <Match when={role().kind === "drag" || role().kind === "decor" || role().kind === "group"}>
        <Plain node={n()} drag hidden={offView()} />
      </Match>
      <Match when={role().kind === "logo" || role().kind === "part"}>
        <div data-skin-node={n().id} aria-hidden="true" data-tauri-drag-region>
          <Children node={n()} />
        </div>
      </Match>
      <Match when={role().kind === "nameField"}>
        <NameField id={n().id}>
          <Children node={n()} />
        </NameField>
      </Match>
      <Match when={role().kind === "minimize"}>
        <TitleButton node={n()} label="Minimize window" onClick={handleMinimize} />
      </Match>
      <Match when={role().kind === "maximize"}>
        <TitleButton node={n()} label="Maximize window" onClick={handleMaximize} />
      </Match>
      <Match when={role().kind === "close"}>
        <TitleButton node={n()} label="Hide window" onClick={handleHide} />
      </Match>
      <Match when={role().kind === "menuBar"}>
        <PackMenuBar id={n().id}>
          <Children node={n()} />
        </PackMenuBar>
      </Match>
      <Match when={role().kind === "menu" && role()}>
        {(r) => (
          <PackMenu id={n().id} menu={(r() as { key: "first" | "tools" | "help" }).key}>
            <Children node={n()} />
          </PackMenu>
        )}
      </Match>
      <Match when={role().kind === "statusText"}>
        <StatusText id={n().id} />
      </Match>
      <Match when={role().kind === "statusList"}>
        <StatusList id={n().id}>
          <Children node={n()} />
        </StatusList>
      </Match>
      <Match when={role().kind === "adArea"}>
        <AdArea id={n().id}>
          <Children node={n()} />
        </AdArea>
      </Match>
      <Match when={role().kind === "resize" && role()}>
        {(r) => <ResizeHandle id={n().id} edge={(r() as { key: Parameters<typeof ResizeHandle>[0]["edge"] }).key} />}
      </Match>
      <Match when={role().kind === "views"}>
        <div data-skin-node={n().id} id="main-content" tabindex="-1">
          <Children node={n()} />
          {!tree.hasFriendsView && <div class="skin-tree-view-content">{tree.views()}</div>}
        </div>
      </Match>
      <Match when={viewKey() !== undefined}>
        <div data-skin-node={n().id} hidden={viewKey() !== SHOWN_VIEW || offView()}>
          <Children node={n()} />
          {role().kind === "view" && viewKey() === SHOWN_VIEW && (
            <div class="skin-tree-view-content">{tree.views()}</div>
          )}
        </div>
      </Match>
      <Match when={role().kind === "sideRail"}>
        {/* StealthX's rail is a launched popup, closed until its toggle opens it (slice S4). */}
        <Plain node={n()} hidden />
      </Match>
    </Switch>
  );
};

/**
 * The window as the skin's component tree (plan SK2-S2, design §5): one
 * `div[data-skin-node]` per served node, nested in the pack's document
 * order, each role bound to its Rekindle feature. Every box comes from the
 * Host's rules under `.skin-tree`; nothing here positions a node.
 */
const SkinTreeWindow: Component<{ shell: SkinShell; views: () => JSX.Element }> = (props) => (
  <TreeCtx.Provider value={{ views: props.views, hasFriendsView: hasFriendsView(props.shell.root) }}>
    <div class="skin-tree">
      <For each={props.shell.root}>{(node) => <SkinNodeView node={node} />}</For>
    </div>
  </TreeCtx.Provider>
);

export default SkinTreeWindow;
