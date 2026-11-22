// The pack's state triggers a window's skin signals fire (plan SK2-A4
// step 3, design §2.5). The Host computes the signals; this only reads
// which of each node's own triggers they fire and what those triggers do
// to a node: `SetState` (the pack's state art; `Disabled` shows its
// disabled art, the button stays), `ShowTile`/`HideTile` (only what the
// pack itself shows or hides) and `GroupPage`. Nothing here decides a
// capability, and nothing is hidden that the pack does not hide.
import type { NodeTrigger, ShellNode, SkinEvent } from "../../ipc/commands/dto_skin_tree";
import type { SkinSignalSet } from "../../ipc/commands/dto_skin_run";

export interface NodeEffect {
  state?: "normal" | "disabled" | "down";
  hidden?: boolean;
  page?: number;
}

export interface SignalEffects {
  /** By node id. */
  nodes: Map<string, NodeEffect>;
}

/** Whether `on` is fired by the window's current signals. */
export function fires(on: SkinEvent, set: SkinSignalSet): boolean {
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

function edit(out: Map<string, NodeEffect>, id: string): NodeEffect {
  let fx = out.get(id);
  if (!fx) {
    fx = {};
    out.set(id, fx);
  }
  return fx;
}

/** Apply one fired trigger of node `id`, in the pack's order (a later
 *  trigger wins, as the engine ran them in order). */
function apply(out: Map<string, NodeEffect>, id: string, t: NodeTrigger): void {
  const run = t.run;
  switch (run.kind) {
    case "setState":
      edit(out, id).state = run.arg;
      break;
    case "showTile":
    case "hideTile":
      edit(out, run.arg.kind === "this" ? id : run.arg.name).hidden = run.kind === "hideTile";
      break;
    case "groupPage":
      edit(out, id).page = run.arg.page;
      break;
    default:
      break;
  }
}

/** Every node's effect under `set`; none before the first set arrives. */
export function signalEffects(root: ShellNode[], set: SkinSignalSet | null): SignalEffects {
  const nodes = new Map<string, NodeEffect>();
  if (!set) return { nodes };
  const walk = (list: ShellNode[]): void => {
    for (const n of list) {
      for (const t of n.triggers ?? []) {
        if (fires(t.on, set)) apply(nodes, n.id, t);
      }
      walk(n.children ?? []);
    }
  };
  walk(root);
  return { nodes };
}
