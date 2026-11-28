// The file transfers (plan E7.2 G12, SK2-G design §4): mirrors
// `rekindle_types::dto::transfer` and `subscription_events::TransferEvent`.
import { hostInvoke } from "../invoke";

/** Where an attachment lives (`TransferTarget`, internally tagged). */
export type TransferTarget = { kind: "channel"; community: string; channelId: string };

export type TransferState = "queued" | "active" | "paused" | "done" | "failed";

/** One transfer (`TransferRow`), as `transfer_list` answers it. */
export interface TransferRow {
  attachmentId: string;
  target: TransferTarget;
  state: TransferState;
  chunksDone: number;
  chunkCount: number;
  bytesDone: number;
  total: number;
  destName: string;
  destPath: string;
  startedAt: number;
  finishedAt: number | null;
}

/** `TransferEvent`, on the buddy list's `transfer-event` channel. */
export type TransferEvent =
  | { stateChanged: { attachmentId: string; state: TransferState } }
  | {
      progress: {
        attachmentId: string;
        chunksDone: number;
        chunkCount: number;
        bytesDone: number;
        total: number;
      };
    }
  | { downloadComplete: { attachmentId: string } }
  | { removed: { attachmentId: string } };

export const transferCommands = {
  list: () => hostInvoke<TransferRow[]>("transfer_list"),
  /** `false` if the user cancelled the save dialog. */
  start: (attachmentId: string, target: TransferTarget) =>
    hostInvoke<boolean>("transfer_start", { attachmentId, target }),
  stop: (attachmentId: string) => hostInvoke<null>("transfer_stop", { attachmentId }),
  /** Shows the finished file in the OS file manager; never opens it. */
  open: (attachmentId: string) => hostInvoke<null>("transfer_open", { attachmentId }),
  remove: (attachmentId: string, deleteFile: boolean) =>
    hostInvoke<null>("transfer_remove", { attachmentId, deleteFile }),
};
