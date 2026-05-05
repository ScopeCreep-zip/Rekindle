import { Component, For, Show, createMemo, createSignal } from "solid-js";
import { communityState } from "../../stores/community.store";
import { handleForwardChannelMessage } from "../../handlers/community.handlers";
import { ICON_CLOSE } from "../../icons";

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

  return (
    <div class="forward-dialog-overlay" onClick={() => !submitting() && props.onClose()}>
      <div class="forward-dialog" onClick={(e) => e.stopPropagation()}>
        <div class="forward-dialog-header">
          <span class="forward-dialog-title">Forward message</span>
          <button
            class="forward-dialog-close"
            onClick={props.onClose}
            disabled={submitting()}
            aria-label="Close forward dialog"
          >
            <span class="nf-icon" aria-hidden="true">{ICON_CLOSE}</span>
          </button>
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
      </div>
    </div>
  );
};

export default ForwardMessageDialog;
