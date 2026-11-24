import { Component, For, Show, createMemo, createSignal } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";
import { communityState } from "../../stores/community.store";
import { handleForwardChannelMessage } from "../../actions/community.actions";
import { Dialog } from "@kobalte/core/dialog";
import { returnFocusToOpener } from "../../utils/dialog_focus";

interface ForwardMessageDialogProps {
  sourceCommunityId: string;
  sourceChannelId: string;
  sourceMessageId: string;
  onClose: () => void;
}

const ForwardMessageDialog: Component<ForwardMessageDialogProps> = (props) => {
  const [destCommunityId, setDestCommunityId] = createSignal(props.sourceCommunityId);
  const [destChannelId, setDestChannelId] = createSignal("");
  const [submitting, setSubmitting] = createSignal(false);

  const communities = createMemo(() => Object.values(communityState.communities));

  const channelsInDest = createMemo(() => {
    const community = communityState.communities[destCommunityId()];
    if (!community) return [];
    return community.channels.filter(
      (ch) =>
        ch.type === "text" || ch.type === "announcement" || ch.type === "dm",
    );
  });

  function isValid(): boolean {
    return destChannelId().length > 0 && !submitting();
  }

  async function handleSubmit(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    if (!isValid()) return;
    setSubmitting(true);
    const ok = await handleForwardChannelMessage(
      props.sourceCommunityId,
      props.sourceChannelId,
      props.sourceMessageId,
      destCommunityId(),
      destChannelId(),
    );
    setSubmitting(false);
    if (ok) props.onClose();
  }

  // Wiring guide §13c: a modal dialog traps focus, Esc and an outside
  // click close it (not while the forward is in flight), and focus goes
  // back to the control that opened it.
  function handleOpenChange(open: boolean): void {
    if (!open && !submitting()) props.onClose();
  }

  return (
    <Dialog open={true} onOpenChange={handleOpenChange} modal={true}>
      <Dialog.Portal>
      <div class="forward-dialog-overlay">
      <Dialog.Content class="forward-dialog" {...returnFocusToOpener()}>
        <div class="forward-dialog-header">
          <Dialog.Title class="forward-dialog-title">Forward message</Dialog.Title>
          <Dialog.CloseButton
            class="forward-dialog-close"
            disabled={submitting()}
            aria-label="Close forward dialog"
          >
            <SkinGlyph name="close" />
          </Dialog.CloseButton>
        </div>
        <form class="forward-dialog-body" onSubmit={(e) => void handleSubmit(e)}>
          <label class="forward-dialog-label">
            <span>Community</span>
            <select
              class="forward-dialog-select"
              value={destCommunityId()}
              onInput={(e) => {
                setDestCommunityId(e.currentTarget.value);
                setDestChannelId("");
              }}
            >
              <For each={communities()}>
                {(c) => <option value={c.id}>{c.name}</option>}
              </For>
            </select>
          </label>
          <label class="forward-dialog-label">
            <span>Channel</span>
            <select
              class="forward-dialog-select"
              value={destChannelId()}
              onInput={(e) => setDestChannelId(e.currentTarget.value)}
            >
              <option value="">Select a channel…</option>
              <For each={channelsInDest()}>
                {(ch) => <option value={ch.id}>#{ch.name}</option>}
              </For>
            </select>
          </label>
          <Show when={channelsInDest().length === 0}>
            <div class="forward-dialog-hint">No text channels available in this community.</div>
          </Show>
          <div class="forward-dialog-actions">
            <button
              type="button"
              class="forward-dialog-btn"
              onClick={props.onClose}
              disabled={submitting()}
            >
              Cancel
            </button>
            <button
              type="submit"
              class="forward-dialog-btn forward-dialog-btn-primary"
              disabled={!isValid()}
            >
              {submitting() ? "Forwarding…" : "Forward"}
            </button>
          </div>
        </form>
      </Dialog.Content>
      </div>
      </Dialog.Portal>
    </Dialog>
  );
};

export default ForwardMessageDialog;
