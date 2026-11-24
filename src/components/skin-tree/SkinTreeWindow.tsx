import { Component, For, JSX, Match, Show, Switch, createContext, createEffect, createSignal, onCleanup, untrack, useContext } from "solid-js";
import { createStore, produce } from "solid-js/store";
import type { EngineMenu, MenuItem, NodeGlyph, NodeLabel, NodeTrigger, ShellNode, SkinAction, SkinEvent, SkinShell, ViewKey } from "../../ipc/commands/dto_skin_tree";
import type { SkinSignalSet } from "../../ipc/commands/dto_skin_run";
import { skinSignals } from "../../stores/skin-signals.store";
import { applyTreeOp, fired, initialTreeState, pageView, type TreeState } from "./signalEffects";
import { dispatchSkinAction, registerShellRunner } from "../../actions/skin.actions";
import { handleOpenDocs, openSettingsTab } from "../buddy-list/MenuBar";
import NameField from "./NameField";
import { actionLabel } from "../../utils/skin_action_words";
import { InBar, PackContextMenu, PackMenu, PackMenuBar } from "./PackMenus";
import { StatusList, StatusText } from "./StatusField";
import AdArea from "./AdArea";
import FilesView from "./FilesView";
import Infoview from "../infoview/Infoview";
import ServersView from "./ServersView";
import { BrandGlyph } from "./BrandGlyphs";
import type { BrandGlyphName } from "../../ipc/commands/dto";

/** What every node of one window's tree reads. */
interface Tree {
  views: () => JSX.Element;
  /** What the Communities view shows (plan SK2-N4); null where a window has none. */
  communities: () => JSX.Element;
  hasFriendsView: boolean;
  state: TreeState;
  /** Each node's parent, for a control group's pages. */
  parent: Map<string, ShellNode>;
  menus: (menu: EngineMenu) => MenuItem[];
  /** Run one trigger's action for a node: a tree operation here, anything
   *  else through the dispatcher. */
  run: (node: string, action: SkinAction) => void;
  /** Edit requests for the name field (the pack's `EditName`). */
  nameEdits: () => number;
  /** An action's accessible name (a tab names the view its page shows). */
  label: (a: SkinAction) => string;
}

const TreeCtx = createContext<Tree>();

function useTree(): Tree {
  const ctx = useContext(TreeCtx);
  if (!ctx) throw new Error("a skin node outside SkinTreeWindow");
  return ctx;
}

function hasFriendsView(nodes: ShellNode[]): boolean {
  return nodes.some(
    (n) => (n.role.kind === "view" && n.role.key === "friends") || hasFriendsView(n.children ?? []),
  );
}

function parents(nodes: ShellNode[], into = new Map<string, ShellNode>()): Map<string, ShellNode> {
  for (const n of nodes) {
    for (const c of n.children ?? []) into.set(c.id, n);
    parents(n.children ?? [], into);
  }
  return into;
}

/** The pointer and keyboard events a node's triggers listen to. */
type PointerEvent = "click" | "doubleClick" | "leftDown" | "rightDown" | "rightClick";

function pointerTriggers(n: ShellNode, on: PointerEvent): NodeTrigger[] {
  return (n.triggers ?? []).filter((t) => t.on.kind === on);
}

function isPointer(on: SkinEvent): on is { kind: PointerEvent } {
  return on.kind === "click" || on.kind === "doubleClick" || on.kind === "leftDown" || on.kind === "rightDown" || on.kind === "rightClick";
}

/** Whether any node under `n` takes a press (its own pointer trigger),
 *  menu titles included: a control inside `n`. */
function holdsControls(n: ShellNode): boolean {
  return (n.children ?? []).some((c) => (c.triggers ?? []).some((t) => isPointer(t.on)) || holdsControls(c));
}

/** Whether a node under `n` takes a press but is no menu title: what an
 *  ARIA menubar may not own (Onyx lays its tab strip inside its MenuBar). */
function holdsNonMenuControls(n: ShellNode): boolean {
  return (n.children ?? []).some(
    (c) =>
      (c.role.kind !== "menuTitle" && pressAction(c) !== undefined) || holdsNonMenuControls(c),
  );
}

/** The node's own press (click or button down, not the window drag or a
 *  resize): what it does, for its accessible name and `data-skin-action`. */
function pressAction(n: ShellNode): SkinAction | undefined {
  return (n.triggers ?? []).find(
    (t) => (t.on.kind === "click" || t.on.kind === "leftDown" || t.on.kind === "doubleClick") && t.run.kind !== "dragWindow" && t.run.kind !== "resize",
  )?.run;
}

function rightMenu(n: ShellNode): EngineMenu | undefined {
  const t = (n.triggers ?? []).find(
    (t) => (t.on.kind === "rightClick" || t.on.kind === "rightDown") && (t.run.kind === "menu" || t.run.kind === "contextMenu"),
  );
  return t && (t.run.kind === "menu" || t.run.kind === "contextMenu") ? t.run.arg : undefined;
}

/** `kind[:key]`: the served role, stable across every pack and window. */
function roleAttr(n: ShellNode): string {
  const r = n.role;
  return "key" in r ? `${r.kind}:${r.key}` : r.kind;
}

/** `kind[:arg]` of an action, its argument when that is a plain name. */
function actionAttr(a: SkinAction | undefined): string | undefined {
  if (!a) return undefined;
  return "arg" in a && typeof a.arg === "string" ? `${a.kind}:${a.arg}` : a.kind;
}


/**
 * The words the Host serves on a node it placed (plan SK2-N4d): over the
 * node's art, centred in `cover` (filled with the skin's ground so the copied
 * art's own word is hidden) or in the whole box. Presses pass through to
 * the node; its accessible name stays the action's.
 */
const NodeLabelView: Component<{ label: NodeLabel }> = (props) => {
  const box = (): JSX.CSSProperties | undefined => {
    const c = props.label.cover;
    return c
      ? { "--label-x": `${c[0]}px`, "--label-y": `${c[1]}px`, "--label-w": `${c[2]}px`, "--label-h": `${c[3]}px` }
      : undefined;
  };
  // A word one size too wide for its box (SK5: "Rekindle" in Symbiosis's
  // and Alloy's 44-45 px titles at 11 px) is drawn a size smaller; a box too
  // narrow even then still ellipsizes. Measured at 11 px, again on resize.
  const [tight, setTight] = createSignal(false);
  const fitText = (el: HTMLSpanElement): void => {
    const measure = (): void => {
      setTight(false);
      queueMicrotask(() => setTight(el.scrollWidth > el.clientWidth + 0.5));
    };
    const ro = new ResizeObserver(measure);
    ro.observe(el.parentElement ?? el);
    onCleanup(() => ro.disconnect());
  };
  return (
    <Show
      when={props.label.glyph}
      fallback={
        <span
          class="skin-node-label"
          data-cover={props.label.cover ? "" : undefined}
          data-tight={tight() ? "" : undefined}
          style={box()}
          aria-hidden="true"
        >
          <span class="skin-node-label-text" ref={fitText}>
            {props.label.text}
          </span>
        </span>
      }
    >
      {(glyph) => (
        <span
          class="skin-node-label skin-node-glyph"
          data-cover={props.label.cover ? "" : undefined}
          style={box()}
          title={props.label.text}
        >
          <NodeGlyphView glyph={glyph()} />
          <span class="sr-only">{props.label.text}</span>
        </span>
      )}
    </Show>
  );
};

/** A built-in glyph (plan SK3), sized by its box: an icon in the current colour, or the Rekindle mark. */
const NodeGlyphView: Component<{ glyph: NodeGlyph }> = (props) => (
  <Switch>
    <Match when={props.glyph === "communities"}>
      {/* Three people: the community mark. */}
      <svg class="skin-node-glyph-svg" viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="12" cy="7" r="3.2" fill="currentColor" />
        <path d="M6.5 19c0-3.2 2.5-5.6 5.5-5.6s5.5 2.4 5.5 5.6z" fill="currentColor" />
        <circle cx="5" cy="9.5" r="2.3" fill="currentColor" opacity="0.8" />
        <path d="M1 18.5c0-2.4 1.8-4.2 4-4.2 1 0 1.9.3 2.6.9-1.2 1-2 2.3-2.3 3.8z" fill="currentColor" opacity="0.8" />
        <circle cx="19" cy="9.5" r="2.3" fill="currentColor" opacity="0.8" />
        <path d="M23 18.5c0-2.4-1.8-4.2-4-4.2-1 0-1.9.3-2.6.9 1.2 1 2 2.3 2.3 3.8z" fill="currentColor" opacity="0.8" />
      </svg>
    </Match>
    {/* The Rekindle mark (plan BR), in full colour, centred in the cover
        with its aspect kept. */}
    <Match when={props.glyph !== "communities" && props.glyph}>
      {(brand) => <BrandGlyph glyph={brand() as BrandGlyphName} />}
    </Match>
  </Switch>
);

/** One node, as the served role draws it and its triggers drive it
 *  (design §2.4, §2.6). */
const SkinNodeView: Component<{ node: ShellNode }> = (props) => {
  const tree = useTree();
  const n = (): ShellNode => props.node;
  const role = (): ShellNode["role"] => props.node.role;
  const press = (): SkinAction | undefined => pressAction(n());
  const fx = () => tree.state.nodes[n().id];
  const disabled = (): boolean => fx()?.state === "disabled";
  const viewKey = (): ViewKey | undefined => {
    const r = role();
    return r.kind === "view" || r.kind === "buttonRow" ? r.key : undefined;
  };
  /** Off its control group's shown page (the pack's own paging). */
  const offPage = (): boolean => {
    const p = tree.parent.get(n().id);
    if (p?.controlGroup === undefined) return false;
    // A page of its own group pages nothing: Symbiosis's FriendsButtonBar is
    // page 0 of TabsButtonsContainer's group 1 and carries group 1 too, and
    // paging its buttons by it left only its first (Chat) shown, Add Friend,
    // Info and Join hidden.
    if (tree.parent.get(p.id)?.controlGroup === p.controlGroup) return false;
    return (p.children ?? []).indexOf(n()) !== (tree.state.pages[p.controlGroup] ?? 0);
  };
  const offView = (): boolean => {
    const s = n().showsWith;
    const v = viewKey();
    return (s !== undefined && s !== tree.state.view) || (v !== undefined && v !== tree.state.view);
  };
  const hidden = (): boolean => fx()?.hidden === true || offPage() || offView();
  const listens = (): string =>
    (n().triggers ?? [])
      .map((t) => t.on)
      .filter(isPointer)
      .map((on) => on.kind)
      .join(" ");
  const inputRole = (): boolean =>
    ["nameField", "statusText", "statusList", "menuBar", "views", "view", "adArea", "infoView"].includes(role().kind);

  /** This node takes a pointer event only when it is the nearest node
   *  that listens (a press on a button is never its frame's window drag). */
  const mine = (e: Event): boolean =>
    e.target instanceof Element && e.target.closest("[data-skin-on]") === e.currentTarget;
  const fire = (on: PointerEvent, e: Event): void => {
    if (!mine(e) || disabled()) return;
    const ts = pointerTriggers(n(), on);
    if (ts.length === 0) return;
    // A served menu opens itself in this event (Kobalte, which must see
    // the event undisturbed); only a menu with no items yet goes to the Host.
    const runs = ts.filter((t) => !((t.run.kind === "menu" || t.run.kind === "contextMenu") && tree.menus(t.run.arg).length > 0));
    if (runs.length === 0) return;
    if (on === "rightClick" || on === "rightDown") e.preventDefault();
    for (const t of runs) tree.run(n().id, t.run);
  };
  // A drag strip is the window's grip, not a control: its double-click
  // maximize and right-click menu are pointer shortcuts for the pack's own
  // buttons and menus. As a button it would hold those controls inside one
  // named "Maximize" (axe nested-interactive, label-content-name-mismatch)
  // and take a focus ring the size of the window.
  // A node holding other controls is no button either (Xfire Classic's
  // logo holds its window menu): a button may hold no control (axe
  // nested-interactive). Its pointer triggers still run.
  // Any node that drags the window on a press is a grip like a drag strip,
  // whatever its role (SpaceCowboy's frame pieces are decor with leftDown →
  // dragWindow and doubleClick → maximize): it rendered as a "Maximize"
  // button, a target and a focus stop the size of the frame.
  const drags = (): boolean => (n().triggers ?? []).some((t) => t.run.kind === "dragWindow");
  const pressable = (): boolean =>
    press() !== undefined && !role().kind.startsWith("menu") && role().kind !== "drag" && !drags() && !holdsControls(n());
  // Inside a menu bar every element is a menu title or presentational
  // (ARIA menubar's required owned elements).
  const inBar = useContext(InBar);

  const attrs: Record<string, unknown> = {
    get "data-skin-node"() {
      return n().id;
    },
    get "data-skin-role"() {
      return roleAttr(n());
    },
    get "data-skin-action"() {
      return actionAttr(press());
    },
    get "data-skin-on"() {
      return listens() || (inputRole() ? "input" : undefined);
    },
    get "data-state"() {
      return fx()?.state;
    },
    // A text field's served inset (plan SK5): its text keeps clear of the
    // pack's controls laid over it.
    get style() {
      const i = n().inset;
      return i ? { "--field-inset-left": `${i.left}px`, "--field-inset-right": `${i.right}px` } : undefined;
    },
    get "aria-disabled"() {
      return disabled() ? "true" : undefined;
    },
    get hidden() {
      return hidden() || undefined;
    },
    onPointerDown: (e: PointerEvent & globalThis.PointerEvent) => fire(e.button === 2 ? "rightDown" : "leftDown", e),
    onClick: (e: MouseEvent) => fire("click", e),
    onDblClick: (e: MouseEvent) => fire("doubleClick", e),
    onContextMenu: (e: MouseEvent) => fire("rightClick", e),
  };
  // The same attributes (getters kept, so they stay reactive) plus a
  // button's role, focus, name and keyboard press.
  const buttonAttrs: Record<string, unknown> = Object.defineProperties({}, Object.getOwnPropertyDescriptors(attrs));
  Object.defineProperties(buttonAttrs, {
    role: { get: () => "button", enumerable: true },
    tabindex: { get: () => (hidden() ? undefined : 0), enumerable: true },
    // The words the node shows are its name (plan SK14, WCAG 2.5.3): a served
    // label names it, else its action; the label is also its tooltip, so words
    // the box cuts short read in full on hover (focus: skin-tree.css).
    "aria-label": { get: () => n().label?.text ?? tree.label(press()!), enumerable: true },
    title: { get: () => n().label?.text, enumerable: true },
    class: { get: () => "skin-tree-button", enumerable: true },
    onKeyDown: {
      value: (e: KeyboardEvent) => {
        if ((e.key === "Enter" || e.key === " ") && !disabled()) {
          e.preventDefault();
          const a = press();
          if (a) tree.run(n().id, a);
        }
      },
      enumerable: true,
    },
  });

  const children = (): JSX.Element => (
    <>
      <Children node={n()} />
      <Show when={n().label}>{(label) => <NodeLabelView label={label()} />}</Show>
    </>
  );
  const menuOf = (): EngineMenu | undefined => {
    const a = press();
    return a && (a.kind === "menu" || a.kind === "contextMenu") && tree.menus(a.arg).length > 0 ? a.arg : undefined;
  };
  const ctxMenu = (): EngineMenu | undefined => {
    const m = rightMenu(n());
    return m && tree.menus(m).length > 0 ? m : undefined;
  };
  const runItem = (item: MenuItem): void => tree.run(n().id, item.run);

  return (
    <Switch fallback={<div {...attrs} role={inBar ? "none" : undefined}>{children()}</div>}>
      <Match when={menuOf()}>
        {(menu) => (
          <PackMenu attrs={attrs} menu={menu()} label={n().label?.text} items={tree.menus(menu())} run={runItem}>
            {children()}
          </PackMenu>
        )}
      </Match>
      <Match when={ctxMenu()}>
        {(menu) => (
          <PackContextMenu attrs={pressable() ? buttonAttrs : attrs} items={tree.menus(menu())} run={runItem}>
            {children()}
          </PackContextMenu>
        )}
      </Match>
      <Match when={role().kind === "nameField"}>
        <NameField attrs={attrs} edit={tree.nameEdits()}>
          {children()}
        </NameField>
      </Match>
      {/* A bar holding other controls is no ARIA menubar (it may own only
       *  menu items): its titles are each their own one-menu bar. */}
      <Match when={role().kind === "menuBar" && !holdsNonMenuControls(n())}>
        <PackMenuBar attrs={attrs}>{children()}</PackMenuBar>
      </Match>
      <Match when={role().kind === "statusText"}>
        <StatusText attrs={attrs} />
      </Match>
      <Match when={role().kind === "statusList"}>
        <StatusList attrs={attrs}>{children()}</StatusList>
      </Match>
      <Match when={role().kind === "adArea"}>
        <AdArea attrs={attrs}>{children()}</AdArea>
      </Match>
      <Match when={role().kind === "views"}>
        <div {...attrs} id="main-content" tabindex="-1">
          {children()}
          {!tree.hasFriendsView && <div class="skin-tree-view-content">{tree.views()}</div>}
        </div>
      </Match>
      <Match when={role().kind === "view" && viewKey() === "friends"}>
        <div {...attrs}>
          {children()}
          <div class="skin-tree-view-content">{tree.views()}</div>
        </div>
      </Match>
      {/* A pack's InfoView (its Info popup or an inline pane, plan E7.15 IV5): the
       *  Host's Infoview for what this window's launcher has selected. */}
      <Match when={role().kind === "infoView"}>
        <div {...attrs}>
          {children()}
          <div class="skin-tree-view-content">
            <Infoview subject={{ type: "selection" }} />
          </div>
        </div>
      </Match>
      <Match when={role().kind === "view" && viewKey() === "communities"}>
        <div {...attrs}>
          {children()}
          <div class="skin-tree-view-content">{tree.communities()}</div>
        </div>
      </Match>
      <Match when={role().kind === "view" && viewKey() === "servers"}>
        <div {...attrs}>
          {children()}
          <div class="skin-tree-view-content">
            <ServersView />
          </div>
        </div>
      </Match>
      <Match when={role().kind === "view" && viewKey() === "files"}>
        <div {...attrs}>
          {children()}
          <div class="skin-tree-view-content">
            <FilesView onExecute={() => tree.run(n().id, { kind: "openDownload" })} />
          </div>
        </div>
      </Match>
      <Match when={pressable()}>
        <div {...buttonAttrs}>{children()}</div>
      </Match>
      {/* Art is hidden from assistive tech, unless it holds a control
       *  (a control under aria-hidden is unreachable, axe aria-hidden-focus). */}
      <Match when={(role().kind === "logo" || role().kind === "part" || role().kind === "decor") && !holdsControls(n())}>
        <div {...attrs} aria-hidden="true">
          {children()}
        </div>
      </Match>
    </Switch>
  );
};

const Children: Component<{ node: ShellNode }> = (props) => (
  <For each={props.node.children ?? []}>{(c) => <SkinNodeView node={c} />}</For>
);

/**
 * The window as the skin's component tree (plan SK2-S2, SK2-A5; design
 * §2.6): one element per served node, nested in the pack's document order.
 * Every box comes from the Host's rules under `.skin-tree`; what a node
 * does is its triggers, run through the one dispatcher.
 */
const SkinTreeWindow: Component<{ shell: SkinShell; views: () => JSX.Element; communities: () => JSX.Element }> = (props) => {
  const [state, setState] = createStore<TreeState>(initialTreeState());
  const [nameEdits, setNameEdits] = createSignal(0);
  const root = (): ShellNode[] => props.shell.root;

  const treeOp = (node: string, action: SkinAction): boolean => {
    let done = false;
    setState(produce((s) => (done = applyTreeOp(s, node, action, root()))));
    return done;
  };
  const run = (node: string, action: SkinAction): void => {
    if (!treeOp(node, action)) void dispatchSkinAction(node, action);
  };

  // The pack's state triggers, run on their events (a signal turning on,
  // a view being shown), in document order.
  let before: { set: SkinSignalSet | null; view: ViewKey } | null = null;
  createEffect(() => {
    const now = { set: skinSignals(), view: state.view };
    if (!now.set) return;
    untrack(() => {
      for (const f of fired(root(), before, now)) treeOp(f.node, f.run);
      before = { set: now.set, view: state.view };
    });
  });

  // The operations the Host hands back that act on this tree.
  onCleanup(
    registerShellRunner((op, node) => {
      switch (op.kind) {
        case "setState":
        case "showTile":
        case "hideTile":
        case "groupPage":
          return treeOp(node, { kind: op.kind, arg: op.arg } as SkinAction);
        case "showView":
          if (treeOp(node, { kind: "showView", arg: op.arg })) return true;
          if (op.arg === "options") {
            openSettingsTab("application");
            return true;
          }
          if (op.arg === "help") {
            handleOpenDocs();
            return true;
          }
          return false;
        case "editName":
          setNameEdits((c) => c + 1);
          return true;
        default:
          return false;
      }
    }),
  );

  const menus = (menu: EngineMenu): MenuItem[] => props.shell.menus.menus.find((m) => m.menu === menu)?.items ?? [];

  return (
    <TreeCtx.Provider
      value={{
        views: props.views,
        communities: props.communities,
        hasFriendsView: hasFriendsView(props.shell.root),
        state,
        parent: parents(props.shell.root),
        menus,
        run,
        nameEdits,
        label: (a) => {
          if (a.kind === "groupPage") {
            const v = pageView(props.shell.root, a.arg.group, a.arg.page);
            if (v) return `Show ${v.replace(/([A-Z])/g, " $1").toLowerCase()}`;
          }
          return actionLabel(a);
        },
      }}
    >
      {/* Whether this window's signal sets have started arriving: the
       *  pack's state art is applied from them, so the real-app suite
       *  waits for "live" before reading a button's state. */}
      <div class="skin-tree" data-skin-signals={skinSignals() ? "live" : "none"}>
        <For each={props.shell.root}>{(node) => <SkinNodeView node={node} />}</For>
      </div>
    </TreeCtx.Provider>
  );
};

export default SkinTreeWindow;
