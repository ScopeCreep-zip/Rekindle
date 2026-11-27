import { eventStreamLive } from "../../ipc/channels";
import { commands } from "../../ipc/commands";
import type { CoCreationSnapshot } from "../../ipc/commands/community";
import {
  applyCoCreationSnapshot,
  coCreationSeq,
  dropCreatorInvite,
  watchCreation,
} from "../../stores/co-creator.store";
import { errorMessage } from "../../utils/error";

// Co-creation (plan E3.2 S7s.3f). Every call is a `hostInvoke`: the Host's
// intent gate (ADR 0019) shows its review, and a cancelled review rejects.
// The Host validates the co-creators (friends, cap, duplicates); the window
// shows its refusal as given.

/// Start a creation with `coCreators` and show its status view. Rejects
/// with the Host's reason, so the form stays open.
export async function handleCreateWithCoCreators(
  name: string,
  coCreators: string[]
): Promise<void> {
  const created = await commands.createCommunityWithCoCreators(name, coCreators);
  watchCreation(created.anchor, created.name, created.pendingCoCreators);
}

/// The Host's co-creation snapshot, or null (logged) when it is not read.
let coCreationTracked = false;

/** Read the Host's co-creation snapshot once this window's event stream is
 *  live; once per window, from whichever co-creation view mounts first. */
export function trackCoCreation(): void {
  if (coCreationTracked) return;
  coCreationTracked = true;
  void eventStreamLive().then(async () => {
    const before = coCreationSeq();
    const snapshot = await handleGetCoCreation();
    if (snapshot) applyCoCreationSnapshot(snapshot, before);
  });
}

async function handleGetCoCreation(): Promise<CoCreationSnapshot | null> {
  try {
    return await commands.getCoCreation();
  } catch (e) {
    console.warn("co-creation snapshot not read:", errorMessage(e));
    return null;
  }
}

/// Remove `identity` from the creation of `anchor`; the Host's error, or null.
export async function handleRemoveCoCreator(
  anchor: string,
  identity: string
): Promise<string | null> {
  try {
    await commands.removeCoCreator(anchor, identity);
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

/// Cancel the creation of `anchor`; the Host's error, or null.
export async function handleCancelCreation(anchor: string): Promise<string | null> {
  try {
    await commands.cancelCommunityCreate(anchor);
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

/// Answer the held co-creator invite to `anchor`; the Host's error, or
/// null once answered (the row is then dropped).
export async function handleAnswerCreatorInvite(
  anchor: string,
  accept: boolean
): Promise<string | null> {
  try {
    await commands.answerCreatorInvite(anchor, accept);
    dropCreatorInvite(anchor);
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}
