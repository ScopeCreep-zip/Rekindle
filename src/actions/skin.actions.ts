// The one skin dispatcher (plan SK2-A5, design §2.6): a pack node's
// trigger runs `dispatchSkinAction`, which does one thing: it asks the Host
// (`runSkinAction`, the table in `rekindle-node/src/host/skin/actions.rs`)
// what the action is. A feature action is the Host's own request, run
// through the intent gate there; anything else comes back as a `SkinRun`
// that this window's shell runs. The frontend decides nothing: the only
// operations run here without asking are the OS window drag and resize,
// which must start inside the pointer event that began them.
import { getCurrentWindow } from "@tauri-apps/api/window";
import { commands } from "../ipc/commands";
import { transferCommands } from "../ipc/commands/dto_transfer";
import type { Edge, SkinAction } from "../ipc/commands/dto_skin_tree";
import type { ShellOp, SkinRun, SkinSelection } from "../ipc/commands/dto_skin_run";
import { addToast } from "../stores/toast.store";
import { callsState } from "../stores/calls.store";
import { handleEndDmCall, handleStartDmCall } from "./calls.actions";
import { handleSetMuted } from "./voice.actions";
import { handleOpenGroupChat } from "./group-dm.actions";
import { handleCopyPublicKey } from "./buddy.actions";
import { handleMaximize, handleMinimize } from "./titlebar.actions";
import { handleQuit, handleReportBug, openSettingsTab } from "../components/buddy-list/MenuBar";

/** Each pack edge's OS resize direction (Tauri `ResizeDirection`). */
const DIRECTION = {
  top: "North",
  bottom: "South",
  left: "West",
  right: "East",
  topLeft: "NorthWest",
  topRight: "NorthEast",
  bottomLeft: "SouthWest",
  bottomRight: "SouthEast",
} as const satisfies Record<Edge, string>;

/**
 * Runs one shell operation for the node whose trigger asked for it; `false`
 * when the operation is not this runner's. A window registers the
 * operations only it can run (the buddy list's panels and its Close to the
 * tray, a tree's own node states); the rest are every window's.
 */
export type ShellRunner = (op: ShellOp, node: string) => boolean;

const runners: ShellRunner[] = [];

/** Register a runner for this window; the returned function removes it. */
export function registerShellRunner(run: ShellRunner): () => void {
  runners.unshift(run);
  return () => {
    const at = runners.indexOf(run);
    if (at >= 0) runners.splice(at, 1);
  };
}

/** The operations every window runs the same way. */
const common: ShellRunner = (op) => {
  switch (op.kind) {
    case "minimize":
      handleMinimize();
      return true;
    case "maximize":
      handleMaximize();
      return true;
    case "dragWindow":
      void getCurrentWindow().startDragging();
      return true;
    case "resize":
      void getCurrentWindow().startResizeDragging(DIRECTION[op.arg]);
      return true;
    case "openChat":
      void commands.openChatWindow(op.arg.friend);
      return true;
    case "openProfile":
      void commands.openProfileWindow(op.arg.friend);
      return true;
    case "chooseDownloadDest":
      // `transfer_start` asks where to save in Rust; no path reaches the webview.
      transferCommands.start(op.arg.attachmentId, op.arg.target).catch((e) => addToast(String(e), "error"));
      return true;
    case "openGroupChat":
      handleOpenGroupChat(op.arg.groupId);
      return true;
    case "launchPopup":
      // The shell opens (or closes, when open) the pack popup beside this
      // window; its place and size come from the Host's shell (popup_place.rs).
      commands.openSkinPopup(op.arg.popup).catch((e) => addToast(String(e), "error"));
      return true;
    case "call":
      runCall(op.arg);
      return true;
    case "showSettings":
      openSettingsTab(op.arg);
      return true;
    case "reportBug":
      handleReportBug();
      return true;
    case "copyKey":
      handleCopyPublicKey();
      return true;
    case "quit":
      handleQuit();
      return true;
    default:
      return false;
  }
};

function runCall(op: Extract<ShellOp, { kind: "call" }>["arg"]): void {
  switch (op.kind) {
    case "voice":
    case "video":
      void handleStartDmCall(op.arg.friend, "", op.kind === "video");
      return;
    case "end": {
      const call = callsState.activeCall ?? callsState.outgoingCall;
      if (call) void handleEndDmCall(call.callId);
      return;
    }
    case "mute":
    case "unmute":
      void handleSetMuted(op.kind === "mute");
      return;
  }
}

/** Run `op` for `node`: this window's runners first, then the common ones. */
function runShellOp(op: ShellOp, node: string): void {
  for (const run of runners) if (run(op, node)) return;
  if (common(op, node)) return;
  // A served operation no runner of this window takes: a wiring gap,
  // said as such rather than dropped.
  addToast(`${op.kind} has no handler in this window`, "error");
}

function isSkinRun(value: unknown): value is SkinRun {
  if (typeof value !== "object" || value === null || !("kind" in value)) return false;
  const kind = (value as { kind: unknown }).kind;
  return (kind === "shell" || kind === "notBuiltYet") && "arg" in value;
}

/**
 * A node's trigger: the OS drag and resize start here, inside the pointer
 * event; every other action is the Host's to resolve.
 */
export async function dispatchSkinAction(node: string, action: SkinAction): Promise<void> {
  if (action.kind === "dragWindow" || action.kind === "resize") {
    runShellOp(action.kind === "resize" ? { kind: "resize", arg: action.arg } : { kind: "dragWindow" }, node);
    return;
  }
  let answer: unknown;
  try {
    answer = await commands.runSkinAction(node, action);
  } catch (e) {
    addToast(String(e), "error");
    return;
  }
  // A feature action answered with its request's value; a feature with
  // no Rekindle operation yet (its plan G row) runs nothing.
  if (!isSkinRun(answer) || answer.kind === "notBuiltYet") return;
  runShellOp(answer.arg, node);
}

/** This window's selection for its skin's selection-dependent actions and
 *  capabilities (the Host keeps it per window, lead decision 48). */
export function sendSkinSelection(selection: SkinSelection | null): Promise<void> {
  return commands.setSkinSelection(selection).then(
    () => undefined,
    (e) => {
      addToast(String(e), "error");
    },
  );
}
