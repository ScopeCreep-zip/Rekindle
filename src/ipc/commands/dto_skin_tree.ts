/// A skin's shell outline (`rekindle_types::dto::skin_tree::SkinShell`,
/// plan SK2-S2): the served component tree's ids, roles and nesting. It
/// carries no box: every position is the Host's emitted rule.

import type { SessionStatus } from "./dto";

export type ViewKey =
  | "friends"
  | "servers"
  | "files"
  | "chatRooms"
  | "pictures"
  | "videos"
  | "notifications";

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
        | "decor";
    };

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
  | "chats"
  | "chatRooms"
  | "servers"
  | "files"
  | "videos"
  | "screenshots"
  | "notifications"
  | "options"
  | "help"
  | "inviteUsersToRoom";

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
        | "openNotifications";
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
  | "uploadSelected";

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
}

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
  window: "main" | "chat";
  /** The popup this outline is (its frame carries `data-skin-popup="<name>"`) and where it sits, else the window's own tree. */
  popup?: ShellPopup;
  root: ShellNode[];
  /** The items of every engine menu the shell's buttons open. */
  menus: MenuRegistry;
}

/** `Anchor` (serde `tag = "kind"`, `content = "offset"`). */
export type NodeAnchor =
  | { kind: "start"; offset: number }
  | { kind: "end"; offset: number }
  | { kind: "centre"; offset: { by: number; before: number; after: number } };

/** `NodeSpan` (serde `tag = "kind"`): one axis of a box, px. */
export type NodeSpan =
  | { kind: "fixed"; at: NodeAnchor; size: number }
  | { kind: "stretch"; start: number; end: number };

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
