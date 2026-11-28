import type { Component, JSX } from "solid-js";

type Attrs = Record<string, unknown>;

/**
 * The parts only some windows' trees lay, given by the window as it gives
 * `views` and `communities` (lead, ACL: a window's code reaches only what it
 * draws). The main window gives its status field, name field and its
 * Servers and Files views; the main window and the pack popups give the
 * Infoview. A node whose part its window does not give draws its art alone.
 */
export interface TreePieces {
  statusText?: Component<{ attrs: Attrs }>;
  statusList?: Component<{ attrs: Attrs; children: JSX.Element }>;
  nameField?: Component<{ attrs: Attrs; edit: number; children: JSX.Element }>;
  infoView?: () => JSX.Element;
  servers?: () => JSX.Element;
  files?: (onExecute: () => void) => JSX.Element;
}
