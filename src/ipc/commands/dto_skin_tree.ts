/// A skin's shell outline (`rekindle_types::dto::skin_tree::SkinShell`,
/// plan SK2-S2): the served component tree's ids, roles and nesting. It
/// carries no box: every position is the Host's emitted rule.

export type ViewKey =
  | "friends"
  | "servers"
  | "files"
  | "chatRooms"
  | "pictures"
  | "videos"
  | "notifications";

export type MenuKey = "first" | "tools" | "help";

export type Edge =
  | "top"
  | "bottom"
  | "left"
  | "right"
  | "topLeft"
  | "topRight"
  | "bottomLeft"
  | "bottomRight";

/** `NodeRole` (serde `tag = "kind"`, `content = "key"`). */
export type NodeRole =
  | { kind: "menu"; key: MenuKey }
  | { kind: "tab" | "knob" | "view" | "buttonRow"; key: ViewKey }
  | { kind: "button"; key: string }
  | { kind: "resize"; key: Edge }
  | { kind: "lamp"; key: string }
  | {
      kind:
        | "drag"
        | "logo"
        | "nameField"
        | "minimize"
        | "maximize"
        | "close"
        | "menuBar"
        | "statusText"
        | "statusList"
        | "adArea"
        | "tabStrip"
        | "views"
        | "columnHeader"
        | "buttonBar"
        | "infoToggle"
        | "infoView"
        | "roomName"
        | "sideRail"
        | "sideRailToggle"
        | "consoleLabel"
        | "packLink"
        | "gameLogo"
        | "systemMenu"
        | "group"
        | "part"
        | "decor";
    };

export interface ShellNode {
  id: string;
  role: NodeRole;
  showsWith?: ViewKey;
  children?: ShellNode[];
}

export interface SkinShell {
  window: "main" | "chat";
  root: ShellNode[];
}
