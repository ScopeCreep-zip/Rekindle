import type { UnlistenFn } from "@tauri-apps/api/event";
import { produce } from "solid-js/store";
import { subscribeTransfers } from "../ipc/channels";
import type { TransferEvent } from "../ipc/commands/dto_transfer";
import { loadTransfers } from "../actions/transfer.actions";
import { setTransfersState, transfersState } from "../stores/transfers.store";

// File transfers (plan E7.2 G12): the Host's list, kept by its events. An
// event for a row this window has not listed (a transfer started while the
// list was unread) re-reads the Host's list instead of inventing the row.

function applyTransferEvent(event: TransferEvent): void {
  if (!transfersState.loaded) return;
  if ("removed" in event) {
    const id = event.removed.attachmentId;
    setTransfersState("order", (order) => order.filter((o) => o !== id));
    setTransfersState("rows", produce((rows) => {
      delete rows[id];
    }));
    return;
  }
  const id = "stateChanged" in event
    ? event.stateChanged.attachmentId
    : "progress" in event
      ? event.progress.attachmentId
      : event.downloadComplete.attachmentId;
  if (!transfersState.rows[id]) {
    void loadTransfers();
    return;
  }
  if ("stateChanged" in event) {
    setTransfersState("rows", id, "state", event.stateChanged.state);
  } else if ("progress" in event) {
    const { chunksDone, chunkCount, bytesDone, total } = event.progress;
    setTransfersState("rows", id, { chunksDone, chunkCount, bytesDone, total });
  } else {
    // The finished row's time and path are the Host's: read them back.
    void loadTransfers();
  }
}

/** The buddy list's transfer events (it alone holds the Downloads view). */
export function subscribeBuddyListTransferEvents(): Promise<UnlistenFn> {
  return subscribeTransfers(applyTransferEvent);
}
