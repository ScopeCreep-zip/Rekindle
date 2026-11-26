/// A skin's shell outline (`rekindle_types::dto::skin_tree::SkinShell`,
/// plan SK2-S2): the served component tree's ids, roles and nesting. It
/// carries no box: every position is the Host's emitted rule.

import type { SessionStatus } from "./dto";

export type ViewKey =
  | "friends"
  | "communities"
  | "servers"
  | "files"
  | "chatRooms"
  | "pictures"
  | "videos"
  | "notifications"
  | "communities";

export type Edge =
  | "top"
  | "bottom"
  | "left"
  | "right"
  | "topLeft"
  | "topRight"
  | "bottomLeft"
  | "bottomRight";

/** `NodeRole` (serde `tag = "kind"`, `content = "key"`; plan SK2-A3: what a
 *  node shows; what it does is its `triggers`). */
export type NodeRole =
  | { kind: "tab" | "knob" | "view" | "buttonRow"; key: ViewKey }
  | { kind: "lamp"; key: string }
  | { kind: "windowButton"; key: WindowButtonKind }
  | { kind: "slot"; key: SlotKey }
  | {
      kind:
        | "drag"
        | "logo"
        | "nameField"
        | "menuTitle"
        | "menuBar"
        | "statusText"
        | "statusList"
        | "adArea"
        | "tabStrip"
        | "views"
        | "columnHeader"
        | "buttonBar"
        | "button"
        | "infoView"
        | "roomName"
        | "sideRail"
        | "consoleLabel"
        | "gameLogo"
        | "group"
        | "part"
        | "decor"
        | "titleBar"
        | "chatTitle"
        | "talkingState";
    };

/** `rekindle_types::dto::skin_tree_role::SlotKey` (plan SK17-2): a chat
 *  window box the shell mounts its own control into. */
export type SlotKey = "typingNotice" | "chatArea" | "chatEdit" | "chatSend" | "infoviewButton";

/** `rekindle_types::dto::skin_tree_role::WindowButtonKind`. */
export type WindowButtonKind = "minimize" | "maximize" | "close";

/** `rekindle_types::dto::skin_action::EngineMenu`. */
export type EngineMenu =
  | "file"
  | "tool"
  | "help"
  | "status"
  | "sys"
  | "cwActions"
  | "cwHelp"
  | "gcSystem"
  | "gcActions"
  | "gcHelp";

/** `EngineView`. */
export type EngineView =
  | "home"
  | "friends"
  | "communities"
  | "chats"
  | "chatRooms"
  | "servers"
  | "files"
  | "videos"
  | "screenshots"
  | "notifications"
  | "options"
  | "help"
  | "inviteUsersToRoom"
  | "communities";

/** `OverlayPart`. */
export type OverlayPart =
  | "fps"
  | "clock"
  | "alert"
  | "webCam"
  | "voiceOverlay"
  | "browser";

/** `SkinSettingsTab`. */
export type SkinSettingsTab =
  | "profile"
  | "privacy"
  | "about"
  | "application"
  | "notifications"
  | "audio"
  | "appearance";

/** `AnimateSpec`. */
export interface AnimateSpec {
  delay: number;
  repeat?: number;
  action?: number;
}

/** `TileRef` (serde `tag = "kind"`, `content = "name"`). */
export type TileRef = { kind: "this" } | { kind: "named"; name: string };

/** `SkinAction` (serde `tag = "kind"`, `content = "arg"`): one engine
 *  action a trigger runs (plan SK2 decision 36). */
export type SkinAction =
  | { kind: "resize"; arg: Edge }
  | { kind: "menu" | "contextMenu"; arg: EngineMenu }
  | { kind: "showView"; arg: EngineView }
  | { kind: "groupPage"; arg: { group: number; page: number } }
  | { kind: "showTile" | "hideTile"; arg: TileRef }
  | { kind: "launchPopup"; arg: { popup: string } }
  | { kind: "animate"; arg: AnimateSpec }
  | { kind: "animationStart"; arg: { group: number; spec: AnimateSpec } }
  | { kind: "animationStop"; arg: { group: number } }
  | { kind: "setState"; arg: "normal" | "disabled" | "down" }
  | { kind: "launchUrl"; arg: { url: string } }
  | { kind: "playSound"; arg: { sound: string } }
  | { kind: "enableOverlay" | "disableOverlay"; arg: OverlayPart }
  | { kind: "showSettings"; arg: SkinSettingsTab }
  | { kind: "setStatus"; arg: SessionStatus }
  | {
      kind:
        | "dragWindow"
        | "minimize"
        | "maximize"
        | "close"
        | "animateStop"
        | "addFriend"
        | "chatFriend"
        | "profileFriend"
        | "viewProfile"
        | "joinFriend"
        | "sendMessage"
        | "voiceChat"
        | "hostVoiceChat"
        | "leaveVoiceChat"
        | "groupVoiceChat"
        | "hostGroupVoiceChat"
        | "leaveGroupVoiceChat"
        | "cancelVoiceChat"
        | "endVoiceChat"
        | "muteSelf"
        | "unmuteSelf"
        | "showVoiceWindow"
        | "hostGameLobby"
        | "joinGameLobby"
        | "leaveGameLobby"
        | "launchGameLobby"
        | "addServer"
        | "refreshServer"
        | "startDownload"
        | "stopDownload"
        | "openDownload"
        | "downloadInfo"
        | "newRoom"
        | "createRoom"
        | "enterRoom"
        | "enterAttachedRoom"
        | "joinRoom"
        | "inviteUsersToRoom"
        | "deleteSelected"
        | "openSelected"
        | "uploadSelected"
        | "searchFor"
        | "showWindow"
        | "hideWindow"
        | "dialogOk"
        | "dialogCancel"
        | "updateClock"
        | "tackWindow"
        | "untackWindow"
        | "openOverlayOptions"
        | "attachedClose"
        | "browseBack"
        | "browseForward"
        | "newChat"
        | "newGroup"
        | "createCommunity"
        | "joinCommunity"
        | "showShortcuts"
        | "reportBug"
        | "logOff"
        | "quit"
        | "copyKey"
        | "editName"
        | "showSafetyNumber"
        | "showGameInvite"
        | "videoChat"
        | "openNotifications"
        | "openCommunity"
        | "leaveCommunity"
        | "startCommunity";
    };

/** `Capability`: what the selection allows (`OnAppCan<X>`). */
export type Capability =
  | "addFriend"
  | "chat"
  | "profile"
  | "join"
  | "voiceChat"
  | "hostVoiceChat"
  | "leaveVoiceChat"
  | "cancelVoiceChat"
  | "muteSelf"
  | "unmuteSelf"
  | "addServer"
  | "refreshServer"
  | "startDownload"
  | "stopDownload"
  | "executeDownload"
  | "hostGameLobby"
  | "joinGameLobby"
  | "launchGameLobby"
  | "leaveGameLobby"
  | "createRoom"
  | "enterRoom"
  | "renameRoom"
  | "delete"
  | "deleteSelected"
  | "openSelected"
  | "uploadSelected"
  | "openCommunity"
  | "leaveCommunity";

/** `AppSignal` (serde `tag = "kind"`, `content = "arg"`). */
export type AppSignal =
  | { kind: "can" | "cannot"; arg: Capability }
  | { kind: "prefEnabled" | "prefDisabled"; arg: OverlayPart }
  | {
      kind:
        | "activeUserTyping"
        | "activeUserNotTyping"
        | "voiceRinging"
        | "userStartedTalking"
        | "userStoppedTalking"
        | "incomingVoiceChat"
        | "outgoingVoiceChat"
        | "startVoiceChat"
        | "endVoiceChat"
        | "speakerOn"
        | "speakerOff"
        | "pushToTalkOn"
        | "pushToTalkOff"
        | "newNotification"
        | "notificationsVisited"
        | "downloadComplete"
        | "xfireUrlDownload"
        | "disconnected"
        | "startingLogin"
        | "startRefreshServers"
        | "stopRefreshServers"
        | "gamePrefBrowserDisabled"
        | "webCamOverlayEnabled"
        | "webCamOverlayDisabled";
    };

/** `SkinEvent` (serde `tag = "kind"`, `content = "arg"`). */
export type SkinEvent =
  | { kind: "show"; arg: EngineView }
  | { kind: "app"; arg: AppSignal }
  | {
      kind:
        | "click"
        | "doubleClick"
        | "leftDown"
        | "rightDown"
        | "rightClick"
        | "activate"
        | "deactivate"
        | "activeMode"
        | "inactiveMode"
        | "tabActivated"
        | "checkboxChecked"
        | "checkboxUnchecked"
        | "tacked"
        | "untacked"
        | "filesFocus"
        | "filesKillFocus";
    };

/** `NodeTrigger`: on `on`, run `run`. */
export interface NodeTrigger {
  on: SkinEvent;
  run: SkinAction;
}

export interface ShellNode {
  id: string;
  role: NodeRole;
  showsWith?: ViewKey;
  /** The pack's `ControlGroupId`: tab controls of one group switch together. */
  controlGroup?: number;
  triggers?: NodeTrigger[];
  children?: ShellNode[];
  /** Words the Host draws on a node it placed (plan SK2-N4d): the Communities tab. */
  label?: NodeLabel;
  /** How far the pack's interactive siblings reach into this text field from each edge at
   *  the window's served minimum (plan SK5): its text keeps inside them. Absent: none do. */
  inset?: NodeInset;
  /** The Host's target box for a small interactive node (plan SK7). */
  target?: NodeTarget;
  /** Who draws an interactive node's disabled look (plan E7.7 CR1): "pack", its art has a
   *  disabled state image; "shell", none, so the shell dims it. Absent on other nodes. */
  disabledBy?: "pack" | "shell";
}

/**
 * `NodeTarget` (plan SK7, WCAG 2.5.8): the node's box is this target box
 * (the Host's stylesheet lays it); `box` is offset from the art box's
 * top-left, `art` is the art's rect inside the box, both at the window's
 * served minimum. `short`: neither 24 px nor the spacing exception holds.
 * `frame`: a window-drag area (no control, so no 2.5.8 rule; plan SK11).
 */
export interface NodeTarget {
  box: [number, number, number, number];
  art: [number, number, number, number];
  rule: "size" | "spacing" | "short" | "frame";
}

/** `NodeInset`: px from the field's left and right edges (plan SK5). */
export interface NodeInset {
  left: number;
  right: number;
  /**
   * The text's maximum width in px from its start (left edge + `left`)
   * when its stop does not move with the field's far edge (plan SK15:
   * Alloy's name field under its fixed status field); absent when `right`
   * holds at every width.
   */
  maxWidth?: number;
}

/** A Host-placed node's words (`NodeLabel`, plan SK2-N4d). */
export interface NodeLabel {
  /** The words to draw: "Communities" for N4. */
  text: string;
  /** [x, y, width, height] in px of the node's own box: the copied art's baked-in word on a
   *  tab, a renamed menu title's usable box (the node less its drawn frame, plan SK5); the
   *  label covers it and lays its text in it. Absent when the art carries no word. */
  cover?: [number, number, number, number];
  /** A built-in glyph drawn instead of text (an icon tab, plan SK3), centred in `cover`
   *  (the source icon's box); `text` is then its accessible name and tooltip. */
  glyph?: NodeGlyph;
  /** The source tab's word style (plan SK15), measured from its art: the words drawn at
   *  `sizePx`/`weight`/`color`, laid in `textBox` (not filled; `cover` hides the copied word).
   *  Absent: the shell's label style. */
  style?: LabelStyle;
}

/** `LabelStyle`: a source tab's word style (plan SK15). */
export interface LabelStyle {
  sizePx: number;
  weight: number;
  /** `#RRGGBB`. */
  color: string;
  /** How the words sit in `textBox`, as the source word sits in its art. */
  align: "start" | "centre";
  /** [x, y, width, height] in px of the node's own box the words are laid in. */
  textBox: [number, number, number, number];
}

/** The built-in glyphs the shell draws (`NodeGlyph`). */
/** `communities`: the icon tab N4 places; `rekindle`: the app mark on a renamed, icon-sized
 *  menu title (plan SK5, XGlow's 24x22 file menu) and the brand mark over a pack's logo in a box
 *  at most 2:1; `rekindleWordmark`: the brand wordmark (flame and word) over a pack's logo or
 *  Xfire decor in a wider box, its cover the whole art box (design `br`). */
export type NodeGlyph = "communities" | "rekindle" | "rekindleWordmark" | "rekindleMarkSmall";

/** `MenuItem`: one item of an engine menu. */
export interface MenuItem {
  label: string;
  run: SkinAction;
  /** A separator above the item. */
  apart?: boolean;
  /** A destructive item, drawn as such. */
  danger?: boolean;
}

/** `EngineMenuItems`: one engine menu's items. */
export interface EngineMenuItems {
  menu: EngineMenu;
  items: MenuItem[];
}

/** `MenuRegistry`: Rekindle's items for each engine menu a pack opens. */
export interface MenuRegistry {
  menus: EngineMenuItems[];
}

export interface SkinShell {
  /** The window this outline lays: the buddy list, a chat window, or a
   *  community window (plan SKW1: composed from the pack's chat window). */
  window: "main" | "chat" | "community";
  /** The popup this outline is (its frame carries `data-skin-popup="<name>"`) and where it sits, else the window's own tree. */
  popup?: ShellPopup;
  root: ShellNode[];
  /** The items of every engine menu the shell's buttons open. */
  menus: MenuRegistry;
  /** The tree-laid window's served minimum, logical px, the frame's outsets included
   *  (plan SK4): what the desktop's fit holds the window to. */
  minWidth: number;
  minHeight: number;
  /** The `skin://` hrefs this tree's own rules draw beyond the shared sheet's
   *  (`SkinApplied.images`), decoded only by a window laying this tree. */
  images: string[];
}

/** `Anchor` (serde `tag = "kind"`, `content = "offset"`), as served: a stored pack's
 *  `after` chain (plan SK15b) is resolved by the Host into `start`/`end`/`centre` before any
 *  tree or popup reaches the shell. */
export type NodeAnchor =
  | { kind: "start"; offset: number }
  | { kind: "end"; offset: number }
  | { kind: "centre"; offset: { by: number; before: number; after: number } }
  | { kind: "after"; offset: { sibling: string; gap: number } };

/** `NodeSpan` (serde `tag = "kind"`): one axis of a box, px. */
export type NodeSpan =
  | { kind: "fixed"; at: NodeAnchor; size: number }
  | { kind: "stretch"; start: number; end: number }
  | { kind: "share"; at: NodeAnchor; permille: number };

/** `PopupAnchor`: the popup's left side `offset` px from the launching window's `edge`. */
export interface PopupAnchor {
  edge: "left" | "right";
  offset: number;
}

/** `ShellPopup`: a popup's name and its placement beside the launching window. */
export interface ShellPopup {
  name: string;
  anchor: PopupAnchor;
  width: number;
  minWidth: number;
  minHeight: number;
  /** Its vertical span in the launching window's height. */
  y: NodeSpan;
}
