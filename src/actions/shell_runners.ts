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
import { handleAcceptGroupCall, handleAcceptIncomingCall, handleEndDmCall, handleEndGroupCall, handleStartDmCall } from "./calls.actions";
import { handleJoinVoice, handleSetMuted } from "./voice.actions";
import { handleOpenGroupChat } from "./group-dm.actions";
import { handleCopyPublicKey } from "./buddy.actions";
import { handleQuit, handleReportBug, openSettingsTab } from "./app_menu.actions";
import { handleLeaveCommunity } from "./community/lifecycle";

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

/** A pack's `VoiceChat`, `HostVoiceChat`, `LeaveVoiceChat` and
 *  `CancelVoiceChat`: a 1:1 call started, a ringing call answered or a
 *  group's joined, a call left or our own ring cancelled, as the Host
 *  named by the window's call (plan CAP1, GV3). Mute is
 *  `muteOps`, a separate group: not every window that calls lays it. */
export function callOps(op: ShellOp): boolean {
  if (op.kind !== "call") return false;
  const call = op.arg;
  switch (call.kind) {
    case "voice":
    case "video":
      void handleStartDmCall(call.arg.friend, "", call.kind === "video");
      return true;
    // A pack's Answer (VoiceChat while a friend's call rings, plan CAP1):
    // accept that call, never place one.
    case "answer":
      void handleAcceptIncomingCall(call.arg.callId);
      return true;
    // A pack's Join (GroupVoiceChat, or VoiceChat in a group window, while
    // the group's call rings): join that call, never start one.
    case "joinGroup":
      void handleAcceptGroupCall(call.arg.callId);
      return true;
    // A pack's Leave/Cancel in a chat window (GV3): end that 1:1 call.
    case "leave":
      void handleEndDmCall(call.arg.callId);
      return true;
    // In a group window: leave the group call, or cancel our own ring.
    case "leaveGroup":
      void handleEndGroupCall(call.arg.callId);
      return true;
    case "end": {
      // The window's 1:1 call (active, or our own ring); with none, say so:
      // the Host asked for an end this window has no call for.
      const active = callsState.activeCall ?? callsState.outgoingCall;
      if (active) void handleEndDmCall(active.callId);
      else addToast("call end has no 1:1 call in this window", "error");
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

/** The buddy list's own: Copy Key, the Communities row's Open and Leave
 *  (plan CB1), and Show Voice (plan E7.2 A: the call
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
    // A community selected in the Communities row drives the pack's Call
    // and Profile too (plan E7.7 HSC1, Xfire's selected-item semantics):
    // Call joins its voice channel; Profile opens its window, where its
    // info lives.
    case "call":
      if (op.arg.kind !== "joinVoice") return false;
      void handleJoinVoice(op.arg.arg.channel, op.arg.arg.community);
      return true;
    case "openCommunityInfo":
      commands.openCommunityWindow(op.arg.community).catch((e) => addToast(errorMessage(e), "error"));
      return true;
    // The Communities row (plan CB1): the selected community's window.
    case "openCommunity":
      commands.openCommunityWindow(op.arg.community).catch((e) => addToast(errorMessage(e), "error"));
      return true;
    case "leaveCommunity":
      // The Host's leave intent asks first (ADR 0019); the row then goes in
      // every window (dropCommunity here, LF1's forgotten in the others).
      void handleLeaveCommunity(op.arg.community);
      return true;
    default:
      return false;
  }
}
