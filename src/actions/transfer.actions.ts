import { transferCommands } from "../ipc/commands/dto_transfer";
import { setTransfersState } from "../stores/transfers.store";
import { addToast } from "../stores/toast.store";

// File transfers (plan E7.2 G12): reading the Host's list. The Downloads view
// calls it when it opens unread; the transfer handlers call it when an event
// names a row this window has not listed.

/** Read the Host's list (`transfer_list`) into the store. */
export async function loadTransfers(): Promise<void> {
  try {
    const rows = await transferCommands.list();
    setTransfersState({
      rows: Object.fromEntries(rows.map((r) => [r.attachmentId, r])),
      order: rows.map((r) => r.attachmentId),
      loaded: true,
    });
  } catch (e) {
    addToast(String(e), "error");
  }
}
