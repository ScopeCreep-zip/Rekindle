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
import type { SkinSignalSet } from "../../ipc/commands/dto_skin_run";

type NodeStateName = "normal" | "disabled" | "down";

interface NodeEffect {
  state?: NodeStateName;
  hidden?: boolean;
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
    case "chatRooms":
    case "servers":
    case "files":
    case "videos":
    case "notifications":
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
          out.push({ node: n.id, run: t.run });
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
 * action is not a tree operation (the caller dispatches it).
 */
export function applyTreeOp(state: TreeState, node: string, run: SkinAction, root: ShellNode[]): boolean {
  switch (run.kind) {
    case "setState":
      state.nodes[node] = { ...state.nodes[node], state: run.arg };
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
      return true;
    }
    case "showView": {
      const v = viewOf(run.arg);
      if (v === undefined) return false;
      state.view = v;
      return true;
    }
    default:
      return false;
  }
}

/** Child `page` of the node that is control group `group`. */
function pageNode(list: ShellNode[], group: number, page: number): ShellNode | undefined {
  for (const n of list) {
    if (n.controlGroup === group) return (n.children ?? [])[page];
    const inner = pageNode(n.children ?? [], group, page);
    if (inner) return inner;
  }
  return undefined;
}
