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
import { type TransferRow, transferCommands } from "../ipc/commands/dto_transfer";
import type { Edge, SkinAction } from "../ipc/commands/dto_skin_tree";
import type { ShellOp, SkinRun, SkinSelection } from "../ipc/commands/dto_skin_run";
import { addToast } from "../stores/toast.store";
import { errorMessage } from "../utils/error";
import { isCancelled } from "../ipc/commands/dto";
import { setLocalSearchScope } from "../stores/local_search.store";
import { callsState } from "../stores/calls.store";
import { handleEndDmCall, handleStartDmCall } from "./calls.actions";
import { handleSetMuted } from "./voice.actions";
import { handleOpenGroupChat } from "./group-dm.actions";
import { handleCopyPublicKey } from "./buddy.actions";
import { handleMaximize, handleMinimize } from "./titlebar.actions";
import { handleQuit, handleReportBug, openSettingsTab } from "../components/buddy-list/MenuBar";
import { handleLeaveCommunity } from "./community/lifecycle";

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
      transferCommands.start(op.arg.attachmentId, op.arg.target).catch((e) => addToast(errorMessage(e), "error"));
      return true;
    case "openGroupChat":
      handleOpenGroupChat(op.arg.groupId);
      return true;
    case "openCommunity":
      commands.openCommunityWindow(op.arg.community).catch((e) => addToast(errorMessage(e), "error"));
      return true;
    case "leaveCommunity":
      // The Host's leave intent asks first (ADR 0019); the row then goes in
      // every window (dropCommunity here, LF1's forgotten in the others).
      void handleLeaveCommunity(op.arg.community);
      return true;
    case "launchPopup":
      // The shell opens (or closes, when open) the pack popup beside this
      // window; its place and size come from the Host's shell (popup_place.rs).
      commands.openSkinPopup(op.arg.popup).catch((e) => addToast(errorMessage(e), "error"));
      return true;
    case "call":
      runCall(op.arg);
      return true;
    case "showCall":
      // The skin's Show Voice (plan E7.2 A): the call window forward, or the
      // community window on the voice channel the user is in.
      if (op.arg.target.kind === "call") {
        commands.openCallWindow(op.arg.target.arg.callId).catch((e) => addToast(errorMessage(e), "error"));
      } else {
        const { community, channel } = op.arg.target.arg;
        commands.openCommunityWindow(community, channel).catch((e) => addToast(errorMessage(e), "error"));
      }
      return true;
    case "openSearch":
      // The skin's Search and Downloads' info (plan E7.2 G11): the local
      // search in the scope the Host picked from this window's view.
      setLocalSearchScope(op.arg.scope);
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

/** An operation the Host addressed to this window from a press in a
 *  window it launched (`subscribeSkinShell`, plan SK2-N4), run as this
 *  window's own press's answer. */
export function runAddressedShellOp(op: ShellOp): void {
  runShellOp(op, "");
}

/** The transfer list in one line: how many, and each one's name and state. */
function transferSummary(rows: TransferRow[]): string {
  if (rows.length === 0) return "No transfers";
  const each = rows.slice(0, 5).map((r) => `${r.destName} (${r.state})`).join(", ");
  return `${rows.length} transfer${rows.length === 1 ? "" : "s"}: ${each}${rows.length > 5 ? ", …" : ""}`;
}

function isSkinRun(value: unknown): value is SkinRun {
  if (typeof value !== "object" || value === null || !("kind" in value)) return false;
  const kind = (value as { kind: unknown }).kind;
  return (kind === "shell" || kind === "notBuiltYet" || kind === "launcher") && "arg" in value;
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
    // The Host's refusal in its words (a CommandError, never "[object
    // Object]"); a review the user cancelled is no error.
    if (!isCancelled(e)) addToast(errorMessage(e), "error");
    return;
  }
  if (isSkinRun(answer)) {
    if (answer.kind === "shell") runShellOp(answer.arg, node);
    // A feature with no Rekindle operation yet (lead decision 49): the
    // button stays, and the window says so.
    else if (answer.kind === "notBuiltYet") addToast(answer.arg.message, "info");
    return;
  }
  // A feature action answered with its request's value. Downloads' info
  // (the transfer list) is shown until its search view (plan G11) takes it.
  if (action.kind === "downloadInfo" && Array.isArray(answer)) addToast(transferSummary(answer as TransferRow[]), "info");
}

/** This window's selection for its skin's selection-dependent actions and
 *  capabilities (the Host keeps it per window, lead decision 48). */
export function sendSkinSelection(selection: SkinSelection | null): Promise<void> {
  return commands.setSkinSelection(selection).then(
    () => undefined,
    (e) => {
      addToast(errorMessage(e), "error");
    },
  );
}
