import { createStore } from "solid-js/store";
import type { TransferRow } from "../ipc/commands/dto_transfer";

/**
 * Our file transfers as the Host lists them (plan E7.2 G12): `transfer_list`
 * once the Downloads view first shows, then kept by `TransferEvent`s. Rows
 * are the Host's; nothing here decides what a transfer may do.
 */
interface TransfersState {
  rows: Record<string, TransferRow>;
  /** The attachment ids in the Host's order (newest first). */
  order: string[];
  loaded: boolean;
}

const [transfersState, setTransfersState] = createStore<TransfersState>({ rows: {}, order: [], loaded: false });

export { transfersState, setTransfersState };
