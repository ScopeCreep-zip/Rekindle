import { Component, Show, createSignal, createEffect } from "solid-js";
import type { Community } from "../../../stores/community.store";
import { handleUpdateCommunityInfo } from "../../../handlers/community.handlers";
import { addToast } from "../../../stores/toast.store";
import { ICON_SAVE, ICON_COPY } from "../../../icons";

interface OverviewTabProps {
  community: Community;
  canManage: boolean;
}

const OverviewTab: Component<OverviewTabProps> = (props) => {
  const [editName, setEditName] = createSignal("");
  const [editDescription, setEditDescription] = createSignal("");
  const [copied, setCopied] = createSignal(false);
  const [savingOverview, setSavingOverview] = createSignal(false);

  createEffect(() => {
    setEditName(props.community.name);
    setEditDescription(props.community.description ?? "");
  });

  function handleCopyId(): void {
    navigator.clipboard.writeText(props.community.id);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }

  async function handleSaveOverview(): Promise<void> {
    const newName = editName().trim();
    const newDesc = editDescription().trim();
    const nameChanged = newName && newName !== props.community.name;
    const descChanged = newDesc !== (props.community.description ?? "");
    if (!nameChanged && !descChanged) return;
    setSavingOverview(true);
    try {
      await handleUpdateCommunityInfo(
        props.community.id,
        nameChanged ? newName : null,
        descChanged ? newDesc : null,
      );
      addToast("Community updated", "success");
    } finally {
      setSavingOverview(false);
    }
  }

  return (
    <div class="settings-section">
      <div class="settings-field">
        <label class="settings-field-label">Community Name</label>
        <Show when={props.canManage} fallback={
          <div class="settings-value">{props.community.name}</div>
        }>
          <input
            class="settings-input"
            type="text"
            value={editName()}
            onInput={(e) => setEditName(e.currentTarget.value)}
          />
        </Show>
      </div>
      <div class="settings-field">
        <label class="settings-field-label">Description</label>
        <Show when={props.canManage} fallback={
          <div class="settings-value">{props.community.description || "No description set."}</div>
        }>
          <textarea
            class="settings-textarea"
            rows={3}
            value={editDescription()}
            onInput={(e) => setEditDescription(e.currentTarget.value)}
            placeholder="Community description..."
          />
        </Show>
      </div>
      <div class="settings-field">
        <label class="settings-field-label">Community ID</label>
        <div class="settings-field-row">
          <span class="settings-value settings-value-mono">{props.community.id}</span>
          <button class="settings-copy-btn" onClick={handleCopyId}>
            <span class="nf-icon">{ICON_COPY}</span> {copied() ? "Copied" : "Copy"}
          </button>
        </div>
      </div>
      <Show when={props.canManage}>
        <div class="settings-field">
          <button class="settings-save-btn" onClick={handleSaveOverview} disabled={savingOverview()}>
            <span class="nf-icon">{ICON_SAVE}</span> {savingOverview() ? "Saving..." : "Save Changes"}
          </button>
        </div>
      </Show>
    </div>
  );
};

export default OverviewTab;
