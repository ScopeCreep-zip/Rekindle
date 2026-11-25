// A skin window's node states (plan SK2-A4 step 3, SK2-A5; design §2.5):
// the pack's triggers run as Xfire's engine ran them, on the event: a
// state trigger runs when its event happens (a signal turning on, a view
// being shown, a press), and the last one run wins. The Host computes the
// signals; this only reads which of each node's own triggers an event
// fires and what those triggers do to the tree: `SetState` (the pack's
// state art; `Disabled` shows its disabled art, the button stays),
// `ShowTile`/`HideTile` (only what the pack itself shows or hides),
// `GroupPage` (the page a control group shows) and `ShowView`. Nothing
// here decides a capability, and nothing is hidden that the pack does not
// hide.
import type { EngineView, NodeTrigger, ShellNode, SkinAction, SkinEvent, ViewKey } from "../../ipc/commands/dto_skin_tree";
import type { DisableCause, SkinSignalSet } from "../../ipc/commands/dto_skin_run";

type NodeStateName = "normal" | "disabled" | "down";

interface NodeEffect {
  state?: NodeStateName;
  hidden?: boolean;
  /** What disabled it: the firing trigger's cause (plan CR1), its reason
   *  read from the window's served `disabledReasons`. */
  cause?: DisableCause;
}

/** One window's tree state: per-node effects, each control group's page,
 *  and the view its views area shows. */
export interface TreeState {
  nodes: Record<string, NodeEffect>;
  pages: Record<number, number>;
  view: ViewKey;
}

export function initialTreeState(): TreeState {
  return { nodes: {}, pages: {}, view: "friends" };
}

/** The tree's view for an engine view, when the tree has views of that kind. */
function viewOf(v: EngineView): ViewKey | undefined {
  switch (v) {
    case "friends":
    case "communities":
    case "chatRooms":
    case "servers":
    case "files":
    case "videos":
    case "notifications":
    case "communities":
      return v;
    case "screenshots":
      return "pictures";
    default:
      return undefined;
  }
}

/** Whether `on` holds under the window's signals and shown view. */
function holds(on: SkinEvent, set: SkinSignalSet | null, view: ViewKey): boolean {
  if (on.kind === "show") return viewOf(on.arg) === view;
  if (!set) return false;
  switch (on.kind) {
    case "activate":
      return set.focused;
    case "deactivate":
      return !set.focused;
    case "app": {
      const s = on.arg;
      switch (s.kind) {
        case "can":
          return set.can.includes(s.arg);
        case "cannot":
          return set.cannot.includes(s.arg);
        case "activeUserTyping":
          return set.userTyping;
        case "activeUserNotTyping":
          return !set.userTyping;
        case "voiceRinging":
          return set.voiceRinging;
        case "userStartedTalking":
          return set.talking;
        case "userStoppedTalking":
          return !set.talking;
        case "disconnected":
          return set.disconnected;
        case "newNotification":
          return set.newNotification;
        case "notificationsVisited":
          return !set.newNotification;
        default:
          return false;
      }
    }
    default:
      return false;
  }
}

/** A state trigger fired by an event: the node that carries it and what it runs. */
export interface Fired {
  node: string;
  run: SkinAction;
  /** The cause behind a state trigger, when its event names one. */
  cause?: DisableCause;
}

/** The cause a state event names: a capability the window cannot use, or our connection down. */
function causeOf(on: SkinEvent): DisableCause | undefined {
  if (on.kind !== "app") return undefined;
  if (on.arg.kind === "cannot") return on.arg.arg;
  if (on.arg.kind === "disconnected") return "disconnected";
  return undefined;
}

/**
 * The state triggers that an event fires, in the pack's document order:
 * each trigger whose event holds now and did not before (`before` null:
 * the window's first signals, so every holding trigger fires).
 */
export function fired(
  root: ShellNode[],
  before: { set: SkinSignalSet | null; view: ViewKey } | null,
  now: { set: SkinSignalSet | null; view: ViewKey },
): Fired[] {
  const out: Fired[] = [];
  const walk = (list: ShellNode[]): void => {
    for (const n of list) {
      for (const t of n.triggers ?? []) {
        if (!stateEvent(t)) continue;
        if (holds(t.on, now.set, now.view) && !(before && holds(t.on, before.set, before.view))) {
          out.push({ node: n.id, run: t.run, cause: causeOf(t.on) });
        }
      }
      walk(n.children ?? []);
    }
  };
  walk(root);
  return out;
}

/** A trigger on a state event, not a pointer press. */
function stateEvent(t: NodeTrigger): boolean {
  return t.on.kind === "activate" || t.on.kind === "deactivate" || t.on.kind === "app" || t.on.kind === "show";
}

/**
 * Apply one tree operation, run for `node`, to `state`; `false` when the
 * Host must also see it (the caller dispatches it): an action that is not
 * a tree operation, or a view switch this tree keeps only for its own art
 * (a popup with no views, whose launcher the Host addresses, SK2-N4).
 */
export function applyTreeOp(state: TreeState, node: string, run: SkinAction, root: ShellNode[], cause?: DisableCause): boolean {
  switch (run.kind) {
    case "setState":
      state.nodes[node] = { ...state.nodes[node], state: run.arg, cause: run.arg === "disabled" ? cause : undefined };
      return true;
    case "showTile":
    case "hideTile": {
      const target = run.arg.kind === "this" ? node : run.arg.name;
      state.nodes[target] = { ...state.nodes[target], hidden: run.kind === "hideTile" };
      return true;
    }
    case "groupPage": {
      state.pages[run.arg.group] = run.arg.page;
      const page = pageNode(root, run.arg.group, run.arg.page);
      if (page && page.role.kind === "view") state.view = page.role.key;
      // A tree with no views (StealthX's popup rail paging its launcher's
      // views, group 1; a pack's control groups are skin-wide, so the rail's
      // own `ButtonArea` pages with them): kept here for the rail's own
      // art, and the Host addresses it to the launcher too (SK2-N4).
      return holdsViews(root);
    }
    case "showView": {
      const v = viewOf(run.arg);
      if (v === undefined) return false;
      // A tree with no views (a popup) keeps the view for its rail's
      // highlight (`Show(view)` triggers), and the Host addresses the
      // switch to its launcher too (SK2-N4).
      state.view = v;
      return holdsViews(root);
    }
    default:
      return false;
  }
}

/** Whether `list` holds the views node. */
function holdsViews(list: ShellNode[]): boolean {
  return list.some((n) => n.role.kind === "views" || holdsViews(n.children ?? []));
}

/** The view a `GroupPage{group, page}` shows, if its page is a view. */
export function pageView(root: ShellNode[], group: number, page: number): ViewKey | undefined {
  const n = pageNode(root, group, page);
  return n?.role.kind === "view" ? n.role.key : undefined;
}

/** The view shown as child `page` of a node of control group `group`: one
 *  group can hold several containers (Classic's group 1 is both its views,
 *  `TabbedControl`, and its button rows, `ButtonArea`), which page together. */
function pageNode(list: ShellNode[], group: number, page: number): ShellNode | undefined {
  for (const n of list) {
    if (n.controlGroup === group) {
      const child = (n.children ?? [])[page];
      if (child?.role.kind === "view") return child;
    }
    const inner = pageNode(n.children ?? [], group, page);
    if (inner) return inner;
  }
  return undefined;
}

/** The served signal set's true flags, space-separated (`data-skin-signal-on`). */
export function signalFlags(set: SkinSignalSet | null): string {
  if (!set) return "";
  const flags: [string, boolean][] = [
    ["focused", set.focused],
    ["userTyping", set.userTyping],
    ["voiceRinging", set.voiceRinging],
    ["talking", set.talking],
    ["disconnected", set.disconnected],
    ["newNotification", set.newNotification],
  ];
  return flags.filter(([, on]) => on).map(([name]) => name).join(" ");
}
