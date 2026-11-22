import { Component, For, Show, createEffect, createSignal, onMount } from "solid-js";
import { sendSkinSelection } from "../../actions/skin.actions";
import type { SkinAction } from "../../ipc/commands/dto_skin_tree";
import type { TransferRow, TransferState } from "../../ipc/commands/dto_transfer";
import { loadTransfers } from "../../handlers/transfer.handlers";
import { transfersState } from "../../stores/transfers.store";
import { formatSize } from "../../utils/formatting";

// The Downloads view (plan E7.2 G12, SK2-G §4): the Host's transfers as
// Xfire's Downloads list. A row press makes it the window's selection
// (`setSkinSelection`), which the pack's Start / Stop / Execute buttons act
// on through the Host's table; a double click is Execute (a reveal in the
// OS file manager, never an open). What a transfer may do is the Host's.

const STATE_TEXT: Record<TransferState, string> = {
  queued: "Queued",
  active: "Downloading",
  paused: "Stopped",
  done: "Done",
  failed: "Failed",
};

function statusText(row: TransferRow): string {
  if (row.state !== "active" || row.total <= 0) return STATE_TEXT[row.state];
  return `${STATE_TEXT.active} ${Math.floor((row.bytesDone * 100) / row.total)}%`;
}

const FilesView: Component<{ run: (action: SkinAction) => void }> = (props) => {
  const [selected, setSelected] = createSignal<string | null>(null);

  onMount(() => {
    if (!transfersState.loaded) void loadTransfers();
  });

  // A selected transfer the Host removed is no longer the selection.
  createEffect(() => {
    const id = selected();
    if (id !== null && !transfersState.rows[id]) {
      setSelected(null);
      void sendSkinSelection(null);
    }
  });

  // Execute runs on the selection, so the Host holds it before the press.
  const select = (row: TransferRow): Promise<void> => {
    setSelected(row.attachmentId);
    return sendSkinSelection({ kind: "transfer", arg: { attachmentId: row.attachmentId, target: row.target } });
  };

  return (
    <div class="files-view" role="listbox" aria-label="Downloads">
      <div class="files-view-head" aria-hidden="true">
        <span>Name</span>
        <span>Size</span>
        <span>Status</span>
      </div>
      <Show when={transfersState.loaded && transfersState.order.length === 0}>
        <div class="files-view-empty">No downloads</div>
      </Show>
      <div class="files-view-rows">
        <For each={transfersState.order}>
          {(id) => {
            const row = (): TransferRow | undefined => transfersState.rows[id];
            return (
              <Show when={row()}>
                {(r) => (
                  <div
                    class="files-view-row"
                    role="option"
                    tabindex="0"
                    aria-selected={selected() === id}
                    data-state={r().state}
                    onClick={() => void select(r())}
                    onDblClick={() => void select(r()).then(() => props.run({ kind: "openDownload" }))}
                    onKeyDown={(e) => {
                      if (e.key === " ") {
                        e.preventDefault();
                        void select(r());
                      } else if (e.key === "Enter") {
                        e.preventDefault();
                        void select(r()).then(() => props.run({ kind: "openDownload" }));
                      }
                    }}
                  >
                    <span class="files-view-name">{r().destName}</span>
                    <span class="files-view-size">{formatSize(r().total)}</span>
                    <span class="files-view-status">{statusText(r())}</span>
                  </div>
                )}
              </Show>
            );
          }}
        </For>
      </div>
    </div>
  );
};

export default FilesView;
