import { Component, createSignal } from "solid-js";
import FilesView from "../skin-tree/FilesView";
import { isCommandError } from "../../ipc/commands/dto";
import type { Capability } from "../../ipc/commands/dto_skin_tree";
import { transferCommands, type TransferRow } from "../../ipc/commands/dto_transfer";
import { skinSignals } from "../../stores/skin-signals.store";
import { transfersState } from "../../stores/transfers.store";
import { addToast } from "../../stores/toast.store";
import { errorMessage } from "../../utils/error";

/**
 * The app's own layout's Downloads tab (Xfire's Downloads; plan E7.2 G12):
 * the same Downloads view a skin's tree shows, with the Start, Stop and
 * Execute a pack would draw. Each acts on the selected download through the
 * Host's transfer command, enabled only while the Host's capabilities for
 * this window and its selection allow it (`startDownload`, `stopDownload`,
 * `executeDownload`).
 */
const DownloadsTab: Component = () => {
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const selected = (): TransferRow | undefined => {
    const id = selectedId();
    return id === null ? undefined : transfersState.rows[id];
  };
  const can = (c: Capability): boolean => !!selected() && (skinSignals()?.can.includes(c) ?? false);
  const why = (c: Capability): string | undefined =>
    can(c) ? undefined : selected() ? "Not available for this download" : "Choose a download first";

  const run = async (what: string, action: () => Promise<unknown>): Promise<void> => {
    try {
      await action();
    } catch (e) {
      if (!(isCommandError(e) && e.type === "cancelled")) addToast(`${what}: ${errorMessage(e)}`, "error");
    }
  };
  const start = (): void => {
    const r = selected();
    if (r && can("startDownload")) void run("Download not started", () => transferCommands.start(r.attachmentId, r.target));
  };
  const stop = (): void => {
    const r = selected();
    if (r && can("stopDownload")) void run("Download not stopped", () => transferCommands.stop(r.attachmentId));
  };
  const execute = (row: TransferRow | undefined = selected()): void => {
    if (row) void run("Download not shown", () => transferCommands.open(row.attachmentId));
  };

  const button = (label: string, c: Capability, press: () => void) => (
    <button
      type="button"
      class="form-btn-secondary downloads-tab-action"
      aria-disabled={can(c) ? undefined : "true"}
      title={why(c)}
      onClick={() => can(c) && press()}
    >
      {label}
    </button>
  );

  return (
    <>
      <FilesView onSelect={(r) => setSelectedId(r?.attachmentId ?? null)} onExecute={(r) => execute(r)} />
      <div class="downloads-tab-actions" role="toolbar" aria-label="Download actions">
        {button("Start", "startDownload", start)}
        {button("Stop", "stopDownload", stop)}
        {button("Execute", "executeDownload", () => execute())}
      </div>
    </>
  );
};

export default DownloadsTab;
