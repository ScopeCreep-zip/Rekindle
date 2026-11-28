// What a served skin node's own data says (plan SK2-A5): its triggers'
// pointer events and press, its right-click menu, its role and action as
// stable attributes, and the tree's parents and views. Pure reads of the
// served tree, shared by the window's nodes (SkinTreeWindow).

import type {
  EngineMenu,
  NodeTrigger,
  ShellNode,
  SkinAction,
  SkinEvent,
} from "../../ipc/commands/dto_skin_tree";

export function hasFriendsView(nodes: ShellNode[]): boolean {
  return nodes.some(
    (n) => (n.role.kind === "view" && n.role.key === "friends") || hasFriendsView(n.children ?? [])
  );
}

export function parents(
  nodes: ShellNode[],
  into = new Map<string, ShellNode>()
): Map<string, ShellNode> {
  for (const n of nodes) {
    for (const c of n.children ?? []) into.set(c.id, n);
    parents(n.children ?? [], into);
  }
  return into;
}

/** The pointer and keyboard events a node's triggers listen to. */
export type PointerEvent = "click" | "doubleClick" | "leftDown" | "rightDown" | "rightClick";

export function pointerTriggers(n: ShellNode, on: PointerEvent): NodeTrigger[] {
  return (n.triggers ?? []).filter((t) => t.on.kind === on);
}

export function isPointer(on: SkinEvent): on is { kind: PointerEvent } {
  return (
    on.kind === "click" ||
    on.kind === "doubleClick" ||
    on.kind === "leftDown" ||
    on.kind === "rightDown" ||
    on.kind === "rightClick"
  );
}

/** Whether any node under `n` takes a press (its own pointer trigger),
 *  menu titles included: a control inside `n`. */
export function holdsControls(n: ShellNode): boolean {
  return (n.children ?? []).some(
    (c) => (c.triggers ?? []).some((t) => isPointer(t.on)) || holdsControls(c)
  );
}

/** Whether a node under `n` takes a press but is no menu title: what an
 *  ARIA menubar may not own (Onyx lays its tab strip inside its MenuBar). */
export function holdsNonMenuControls(n: ShellNode): boolean {
  return (n.children ?? []).some(
    (c) => (c.role.kind !== "menuTitle" && pressAction(c) !== undefined) || holdsNonMenuControls(c)
  );
}

/** The node's own press (click or button down, not the window drag or a
 *  resize): what it does, for its accessible name and `data-skin-action`. */
export function pressAction(n: ShellNode): SkinAction | undefined {
  return (n.triggers ?? []).find(
    (t) =>
      (t.on.kind === "click" || t.on.kind === "leftDown" || t.on.kind === "doubleClick") &&
      t.run.kind !== "dragWindow" &&
      t.run.kind !== "resize"
  )?.run;
}

export function rightMenu(n: ShellNode): EngineMenu | undefined {
  const t = (n.triggers ?? []).find(
    (t) =>
      (t.on.kind === "rightClick" || t.on.kind === "rightDown") &&
      (t.run.kind === "menu" || t.run.kind === "contextMenu")
  );
  return t && (t.run.kind === "menu" || t.run.kind === "contextMenu") ? t.run.arg : undefined;
}

/** `kind[:key]`: the served role, stable across every pack and window. */
export function roleAttr(n: ShellNode): string {
  const r = n.role;
  return "key" in r ? `${r.kind}:${r.key}` : r.kind;
}

/** `kind[:arg]` of an action, its argument when that is a plain name. */
export function actionAttr(a: SkinAction | undefined): string | undefined {
  if (!a) return undefined;
  return "arg" in a && typeof a.arg === "string" ? `${a.kind}:${a.arg}` : a.kind;
}
