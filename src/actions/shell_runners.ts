// The skin's shell operations, in groups a window registers by what its
// trees serve (the built-in packs' triggers per window kind, and the engine
// menus their menu titles open). A window registers only the groups its
// trees can lay, so each window's code reaches, and is granted, only the
// commands it can be asked to run (`src-tauri/tests/capability_policy.rs`).
// An operation no registered group takes still answers in the window: the
// dispatcher's "has no handler in this window" (skin.actions runShellOp),
// never a silent drop, since an imported pack may lay what no built-in does.
import { commands } from "../ipc/commands";
import { transferCommands } from "../ipc/commands/dto_transfer";
import type { ShellOp } from "../ipc/commands/dto_skin_run";
import { addToast } from "../stores/toast.store";
import { errorMessage } from "../utils/error";
import { callsState } from "../stores/calls.store";
import { handleEndDmCall, handleStartDmCall } from "./calls.actions";
import { handleSetMuted } from "./voice.actions";
import { handleOpenGroupChat } from "./group-dm.actions";
import { handleCopyPublicKey } from "./buddy.actions";
import { handleQuit, handleReportBug, openSettingsTab } from "./app_menu.actions";

/** The File, Tool and Help menus' own items: Settings on a tab, Report Bug,
 *  Exit (the buddy list, chat windows and the popups that carry them). */
export function menuOps(op: ShellOp): boolean {
  switch (op.kind) {
    case "showSettings":
      openSettingsTab(op.arg);
      return true;
    case "reportBug":
      handleReportBug();
      return true;
    case "quit":
      handleQuit();
      return true;
    default:
      return false;
  }
}

/** A pack's `LaunchPopup`: the shell opens (or closes, when open) the pack
 *  popup beside this window; its place and size come from the Host's shell
 *  (popup_place.rs). */
export function popupOps(op: ShellOp): boolean {
  if (op.kind !== "launchPopup") return false;
  commands.openSkinPopup(op.arg.popup).catch((e) => addToast(errorMessage(e), "error"));
  return true;
}

/** A pack's `ProfileFriend`: the selected friend's profile window. */
export function friendOps(op: ShellOp): boolean {
  if (op.kind !== "openProfile") return false;
  void commands.openProfileWindow(op.arg.friend);
  return true;
}

/** A pack's `ChatFriend` and `EnterRoom`: a conversation's window. */
export function chatOpenOps(op: ShellOp): boolean {
  switch (op.kind) {
    case "openChat":
      void commands.openChatWindow(op.arg.friend);
      return true;
    case "openGroupChat":
      handleOpenGroupChat(op.arg.groupId);
      return true;
    default:
      return false;
  }
}

/** A pack's `StartDownload`: `transfer_start` asks where to save in Rust;
 *  no path reaches the webview. */
export function transferOps(op: ShellOp): boolean {
  if (op.kind !== "chooseDownloadDest") return false;
  transferCommands.start(op.arg.attachmentId, op.arg.target).catch((e) => addToast(errorMessage(e), "error"));
  return true;
}

/** A pack's `VoiceChat` and `LeaveVoiceChat`: a 1:1 call started with the
 *  friend the Host named, or the active (or our ringing) call ended. Mute is
 *  `muteOps`, a separate group: not every window that calls lays it. */
export function callOps(op: ShellOp): boolean {
  if (op.kind !== "call") return false;
  const call = op.arg;
  switch (call.kind) {
    case "voice":
    case "video":
      void handleStartDmCall(call.arg.friend, "", call.kind === "video");
      return true;
    case "end": {
      const active = callsState.activeCall ?? callsState.outgoingCall;
      if (active) void handleEndDmCall(active.callId);
      return true;
    }
    default:
      return false;
  }
}

/** A pack's `MuteSelf` and `UnmuteSelf`. */
export function muteOps(op: ShellOp): boolean {
  if (op.kind !== "call" || (op.arg.kind !== "mute" && op.arg.kind !== "unmute")) return false;
  void handleSetMuted(op.arg.kind === "mute");
  return true;
}

/** The buddy list's own: Copy Key, and Show Voice (plan E7.2 A: the call
 *  window forward, or the community window on the voice channel the user
 *  is in), the Host's `ShowVoiceWindow`, which only the main window lays. */
export function mainOps(op: ShellOp): boolean {
  switch (op.kind) {
    case "copyKey":
      handleCopyPublicKey();
      return true;
    case "showCall":
      if (op.arg.target.kind === "call") {
        commands.openCallWindow(op.arg.target.arg.callId).catch((e) => addToast(errorMessage(e), "error"));
      } else {
        const { community, channel } = op.arg.target.arg;
        commands.openCommunityWindow(community, channel).catch((e) => addToast(errorMessage(e), "error"));
      }
      return true;
    default:
      return false;
  }
}
