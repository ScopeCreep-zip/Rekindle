// What a skin action does (plan SK2-A4, design §2.2): mirrors
// `rekindle_types::dto::skin_run`. The Host decides; the shell only runs
// the operation it is handed.
import type { Capability, EngineMenu, SkinAction } from "./dto_skin_tree";

type ArgOf<K extends SkinAction["kind"]> = Extract<SkinAction, { kind: K; arg: unknown }>["arg"];

export type SkinWindow = "main" | "chat";

/** `SkinSignalSet`: one window's state signals, computed by the Host
 *  (design §2.5); the window applies the pack triggers they fire. */
export interface SkinSignalSet {
  focused: boolean;
  can: Capability[];
  cannot: Capability[];
  userTyping: boolean;
  voiceRinging: boolean;
  talking: boolean;
  disconnected: boolean;
  newNotification: boolean;
}

export type SkinSelection = { kind: "friend"; arg: { publicKey: string } };

export type ShellPanel =
  | "addFriend"
  | "newChat"
  | "newGroup"
  | "startGroupCall"
  | "createCommunity"
  | "joinCommunity"
  | "shortcuts"
  | "notifications";

export type CallOp =
  | { kind: "voice"; arg: { friend: string } }
  | { kind: "video"; arg: { friend: string } }
  | { kind: "end" }
  | { kind: "mute" }
  | { kind: "unmute" };

export type ShellOp =
  | { kind: "dragWindow" }
  | { kind: "minimize" }
  | { kind: "maximize" }
  | { kind: "close" }
  | { kind: "resize"; arg: ArgOf<"resize"> }
  | { kind: "openMenu"; arg: { menu: EngineMenu; context: boolean } }
  | { kind: "showView"; arg: ArgOf<"showView"> }
  | { kind: "groupPage"; arg: ArgOf<"groupPage"> }
  | { kind: "showTile"; arg: ArgOf<"showTile"> }
  | { kind: "hideTile"; arg: ArgOf<"hideTile"> }
  | { kind: "launchPopup"; arg: ArgOf<"launchPopup"> }
  | { kind: "animate"; arg: ArgOf<"animate"> }
  | { kind: "animateStop" }
  | { kind: "animationStart"; arg: ArgOf<"animationStart"> }
  | { kind: "animationStop"; arg: ArgOf<"animationStop"> }
  | { kind: "setState"; arg: ArgOf<"setState"> }
  | { kind: "playSound"; arg: ArgOf<"playSound"> }
  | { kind: "openPanel"; arg: ShellPanel }
  | { kind: "openChat"; arg: { friend: string } }
  | { kind: "openProfile"; arg: { friend: string } }
  | { kind: "openSafetyNumber"; arg: { friend: string } }
  | { kind: "openGameInvite"; arg: { friend: string } }
  | { kind: "call"; arg: CallOp }
  | { kind: "showSettings"; arg: ArgOf<"showSettings"> }
  | { kind: "reportBug" }
  | { kind: "copyKey" }
  | { kind: "editName" }
  | { kind: "quit" };

export type SkinRun =
  | { kind: "shell"; arg: ShellOp }
  | { kind: "notBuiltYet"; arg: { action: SkinAction } };
