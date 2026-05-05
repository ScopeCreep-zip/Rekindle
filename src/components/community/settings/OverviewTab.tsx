import { Component, Show, createSignal, createEffect } from "solid-js";
import type { Community } from "../../../stores/community.store";
import { handleUpdateCommunityInfo } from "../../../handlers/community.handlers";
import { addToast } from "../../../stores/toast.store";
import { ICON_SAVE, ICON_COPY } from "../../../icons";
import FormField from "../../common/FormField";

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

  async function handleCopyId(): Promise<void> {
    try {
      await navigator.clipboard.writeText(props.community.id);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      addToast("Failed to copy — clipboard access denied", "error");
    }
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
      <FormField label="Community Name">
        <Show when={props.canManage} fallback={
          <div class="settings-value">{props.community.name}</div>
        }>
          <input
            class="form-input"
            type="text"
            value={editName()}
            onInput={(e) => setEditName(e.currentTarget.value)}
          />
        </Show>
      </FormField>
      <FormField label="Description">
        <Show when={props.canManage} fallback={
          <div class="settings-value">{props.community.description || "No description set."}</div>
        }>
          <textarea
            class="form-textarea"
            rows={3}
            value={editDescription()}
            onInput={(e) => setEditDescription(e.currentTarget.value)}
            placeholder="Community description..."
          />
        </Show>
      </FormField>
      <FormField label="Community ID">
        <div class="form-field-row">
          <span class="settings-value settings-value-mono">{props.community.id}</span>
          <button class="settings-copy-btn" onClick={handleCopyId}>
            <span class="nf-icon">{ICON_COPY}</span> {copied() ? "Copied" : "Copy"}
          </button>
        </div>
      </FormField>
      <Show when={props.canManage}>
        <div class="settings-actions-sticky">
          <button class="form-btn-primary" onClick={handleSaveOverview} disabled={savingOverview()}>
            <span class="nf-icon">{ICON_SAVE}</span> {savingOverview() ? "Saving..." : "Save Changes"}
          </button>
        </div>
      </Show>
    </div>
  );
};

export default OverviewTab;
