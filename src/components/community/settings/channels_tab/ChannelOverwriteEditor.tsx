import { Component, For, Show, createSignal } from "solid-js";
import SkinGlyph from "../../../common/SkinGlyph";
import type { Role } from "../../../../stores/community.store";
import { commands } from "../../../../ipc/commands";
import { hasBits, togglePermBit } from "../../../../ipc/permissions";
import { usePermissionCatalogue } from "../../../../stores/permission_catalogue";
import { errorMessage } from "../../../../utils/error";
import { addToast } from "../../../../stores/toast.store";

interface ChannelOverwriteEditorProps {
  communityId: string;
  channelId: string;
  channelName: string;
  roles: Role[];
}

const ChannelOverwriteEditor: Component<ChannelOverwriteEditorProps> = (props) => {
  const [overwriteTargetType, setOverwriteTargetType] = createSignal("role");
  const [overwriteTargetId, setOverwriteTargetId] = createSignal("");
  const [overwriteAllow, setOverwriteAllow] = createSignal(0n);
  const [overwriteDeny, setOverwriteDeny] = createSignal(0n);
  // The permissions the grid offers: the Host's catalogue (contract PN1).
  const catalogue = usePermissionCatalogue();

  async function handleSaveOverwrite(): Promise<void> {
    const targetId = overwriteTargetId();
    if (!targetId) return;
    try {
      await commands.setChannelOverwrite(
        props.communityId,
        props.channelId,
        overwriteTargetType(),
        targetId,
        Number(overwriteAllow()),
        Number(overwriteDeny()),
      );
      addToast(`Permission overwrite saved for #${props.channelName}`, "success");
    } catch (e) {
      console.error("Failed to save overwrite:", e);
      addToast("Failed to save overwrite", "error");
    }
  }

  async function handleDeleteOverwrite(): Promise<void> {
    const targetId = overwriteTargetId();
    if (!targetId) return;
    try {
      await commands.deleteChannelOverwrite(
        props.communityId,
        props.channelId,
        overwriteTargetType(),
        targetId,
      );
      setOverwriteAllow(0n);
      setOverwriteDeny(0n);
      addToast(`Permission overwrite removed for #${props.channelName}`, "success");
    } catch (e) {
      console.error("Failed to delete overwrite:", e);
      addToast("Failed to delete overwrite", "error");
    }
  }

  return (
    <div class="overwrite-editor">
      <div class="form-field-row">
        <select
          class="form-select"
          value={overwriteTargetType()}
          onChange={(e) => {
            setOverwriteTargetType(e.currentTarget.value);
            setOverwriteTargetId("");
          }}
        >
          <option value="role">Role</option>
        </select>
        <select
          class="form-select"
          value={overwriteTargetId()}
          onChange={(e) => setOverwriteTargetId(e.currentTarget.value)}
        >
          <option value="">Select target...</option>
          <For each={props.roles}>
            {(role) => (
              <option value={String(role.id)}>{role.name}</option>
            )}
          </For>
        </select>
      </div>
      <Show when={overwriteTargetId()}>
        <Show when={catalogue.error}>
          <div class="settings-hint" role="status">Couldn't load the permission list: {errorMessage(catalogue.error)}</div>
        </Show>
        <Show when={catalogue.loading}>
          <div class="settings-hint" role="status">Loading permissions…</div>
        </Show>
        <div class="overwrite-perm-grid">
          <span class="overwrite-perm-header">Permission</span>
          <span class="overwrite-perm-header">Allow</span>
          <span class="overwrite-perm-header">Deny</span>
          <For each={catalogue() ?? []}>
            {(category) => (
              <For each={category.permissions}>
                {(perm) => (
                  <>
                    <span>{perm.label}</span>
                    <input
                      type="checkbox"
                      class="role-picker-checkbox"
                      checked={hasBits(overwriteAllow(), perm.bit)}
                      onChange={() => {
                        setOverwriteAllow(togglePermBit(overwriteAllow(), perm.bit));
                        if (hasBits(overwriteDeny(), perm.bit)) {
                          setOverwriteDeny(togglePermBit(overwriteDeny(), perm.bit));
                        }
                      }}
                    />
                    <input
                      type="checkbox"
                      class="role-picker-checkbox"
                      checked={hasBits(overwriteDeny(), perm.bit)}
                      onChange={() => {
                        setOverwriteDeny(togglePermBit(overwriteDeny(), perm.bit));
                        if (hasBits(overwriteAllow(), perm.bit)) {
                          setOverwriteAllow(togglePermBit(overwriteAllow(), perm.bit));
                        }
                      }}
                    />
                  </>
                )}
              </For>
            )}
          </For>
        </div>
        <div class="form-field-row">
          <button class="form-btn-primary" onClick={handleSaveOverwrite}>
            <SkinGlyph name="save" /> Save Overwrite
          </button>
          <button class="form-btn-danger" onClick={handleDeleteOverwrite}>
            <SkinGlyph name="delete" /> Remove Overwrite
          </button>
        </div>
      </Show>
    </div>
  );
};

export default ChannelOverwriteEditor;
