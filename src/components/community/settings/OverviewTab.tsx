import { type Component, Show, createSignal, createEffect, createUniqueId } from "solid-js";
import SkinGlyph from "../../common/SkinGlyph";
import type { Community } from "../../../stores/community.store";
import { handleUpdateCommunityInfo } from "../../../actions/community.actions";
import { addToast } from "../../../stores/toast.store";
import FormField from "../../common/FormField";
import SuccessorSection from "./SuccessorSection";
import CommunityDeviceStatus from "../CommunityDeviceStatus";
import MyNameSection from "./MyNameSection";

interface OverviewTabProps {
  community: Community;
  /** Why we may not edit the community, in the Host's words, or null (never-hide). */
  manageDenied: string | null;
}

const OverviewTab: Component<OverviewTabProps> = (props) => {
  const communityNameId = createUniqueId();
  const descriptionId = createUniqueId();
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
        descChanged ? newDesc : null
      );
      addToast("Community updated", "success");
    } finally {
      setSavingOverview(false);
    }
  }

  return (
    <div class="settings-section">
      <FormField label="Community Name" controlId={communityNameId}>
        <input
          id={communityNameId}
          class="form-input"
          type="text"
          value={editName()}
          disabled={props.manageDenied !== null}
          title={props.manageDenied ?? undefined}
          aria-description={props.manageDenied ?? undefined}
          onInput={(e) => setEditName(e.currentTarget.value)}
        />
      </FormField>
      <FormField label="Description" controlId={descriptionId}>
        <textarea
          id={descriptionId}
          class="form-textarea"
          rows={3}
          value={editDescription()}
          disabled={props.manageDenied !== null}
          title={props.manageDenied ?? undefined}
          aria-description={props.manageDenied ?? undefined}
          onInput={(e) => setEditDescription(e.currentTarget.value)}
          placeholder="Community description..."
        />
      </FormField>
      <FormField label="Community ID">
        <div class="form-field-row">
          <span class="settings-value settings-value-mono">{props.community.id}</span>
          <button type="button" class="settings-copy-btn" onClick={handleCopyId}>
            <SkinGlyph name="copy" /> {copied() ? "Copied" : "Copy"}
          </button>
        </div>
      </FormField>
      <MyNameSection communityId={props.community.id} />
      <SuccessorSection communityId={props.community.id} />
      <div class="settings-section-title">This device</div>
      <CommunityDeviceStatus communityId={props.community.id} />
      <div class="settings-actions-sticky">
        <button
          type="button"
          class="form-btn-primary"
          onClick={handleSaveOverview}
          disabled={savingOverview() || props.manageDenied !== null}
          title={props.manageDenied ?? undefined}
          aria-description={props.manageDenied ?? undefined}
        >
          <SkinGlyph name="save" /> {savingOverview() ? "Saving..." : "Save Changes"}
        </button>
      </div>
    </div>
  );
};

export default OverviewTab;
