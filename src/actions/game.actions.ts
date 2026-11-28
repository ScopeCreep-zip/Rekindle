import { commands } from "../ipc/commands";
import { isCommandError } from "../ipc/commands/dto";
import type { JoinSource } from "../ipc/commands/types";
import { addToast } from "../stores/toast.store";
import { errorMessage } from "../utils/error";

/**
 * Launch `gameId` at `server` through the Host's review (ADR 0019, plan
 * E7.15 IV3). True once launched. A review the user cancelled shows
 * nothing; a refusal shows the Host's message.
 */
export async function handleLaunchGame(
  gameId: number,
  server: string | null,
  source: JoinSource
): Promise<boolean> {
  try {
    await commands.launchGame(gameId, server, source);
    return true;
  } catch (e) {
    if (!(isCommandError(e) && e.type === "cancelled"))
      addToast(`Game not launched: ${errorMessage(e)}`, "error");
    return false;
  }
}
