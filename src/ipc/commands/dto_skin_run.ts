// What a skin action does (plan SK2-A4, design §2.2): mirrors
// `rekindle_types::dto::skin_run`. The Host decides; the shell only runs
// the operation it is handed.
import type { Capability, EngineMenu, SkinAction } from "./dto_skin_tree";
import type { LocalSearchScope } from "./dto_local_search";
import type { TransferTarget } from "./dto_transfer";

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

export type SkinSelection =
  | { kind: "friend"; arg: { publicKey: string } }
  | { kind: "server"; arg: { gameId: number; address: string } }
  | { kind: "transfer"; arg: { attachmentId: string; target: TransferTarget } }
  | { kind: "media"; arg: { path: string } }
  | { kind: "group"; arg: { groupId: string } }
  /** A friend group's header (plan E7.15 IV5); null is the default group. */
  | { kind: "friendGroup"; arg: { groupId: number | null } };

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
  | { kind: "newRoom"; arg: { peer: string | null } }
  | { kind: "openGroupChat"; arg: { groupId: string } }
  | { kind: "inviteToRoom"; arg: { groupId: string } }
  | { kind: "call"; arg: CallOp }
  | { kind: "showSettings"; arg: ArgOf<"showSettings"> }
  /** Open the local search in `scope`, the Host's pick from the window's view (plan E7.2 G11). */
  | { kind: "openSearch"; arg: { scope: LocalSearchScope } }
  /** A never-fetched attachment: run `transfer_start` for it, which asks
   *  where to save in Rust (plan E7.2 G12); no path reaches the webview. */
  | {
      kind: "chooseDownloadDest";
      arg: { attachmentId: string; target: TransferTarget; suggestedName: string };
    }
  | { kind: "reportBug" }
  | { kind: "copyKey" }
  | { kind: "editName" }
  | { kind: "quit" };

export type SkinRun =
  | { kind: "shell"; arg: ShellOp }
  | { kind: "notBuiltYet"; arg: { action: SkinAction } }
  /** The operation ran in the window labelled `window` (a view switch
   *  pressed in a popup runs in its launcher, plan SK2-N4); this window
   *  runs nothing. */
  | { kind: "launcher"; arg: { window: string } };
