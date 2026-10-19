import { commands } from "../ipc/commands";
import { isCommandError } from "../ipc/commands/dto";
import { errorMessage } from "../utils/error";

/// Mark `peerKey`'s identity key verified, or withdraw it, through the
/// Host's review (TrustChanging, plan E2.3): the review states the safety
/// number, and its checkbox is the user saying they compared it. Returns
/// null when it ran or the user cancelled the review, else the error for
/// the caller's toast.
export async function handleSetVerified(peerKey: string, verified: boolean): Promise<string | null> {
  try {
    if (verified) {
      await commands.markIdentityVerified(peerKey);
    } else {
      await commands.clearIdentityVerification(peerKey);
    }
    return null;
  } catch (e) {
    // 499: the user cancelled the review (`cancelledError`).
    if (isCommandError(e) && e.type === "failed" && e.data.code === 499) return null;
    return errorMessage(e);
  }
}
