import { type Component, createSignal, For, Show } from "solid-js";
import { commands } from "../../../../ipc/commands";
import { PERMISSION_CATEGORIES, togglePermBit } from "../../../../ipc/permissions";
import type { Role } from "../../../../stores/community.store";
import { addToast } from "../../../../stores/toast.store";
import SkinGlyph from "../../../common/SkinGlyph";

interface ChannelOverwriteEditorProps {
  communityId: string;
  channelId: string;
  channelName: string;
  roles: Role[];
}

// Raw bit check for overwrite grid (no admin bypass — overwrites are explicit).
function hasPerm(perms: bigint, bit: bigint): boolean {
  return (perms & bit) !== 0n;
}

const ChannelOverwriteEditor: Component<ChannelOverwriteEditorProps> = (props) => {
  const [overwriteTargetType, setOverwriteTargetType] = createSignal("role");
  const [overwriteTargetId, setOverwriteTargetId] = createSignal("");
  const [overwriteAllow, setOverwriteAllow] = createSignal(0n);
  const [overwriteDeny, setOverwriteDeny] = createSignal(0n);

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
        Number(overwriteDeny())
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
        targetId
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
            {(role) => <option value={String(role.id)}>{role.name}</option>}
          </For>
        </select>
      </div>
      <Show when={overwriteTargetId()}>
        <div class="overwrite-perm-grid">
          <span class="overwrite-perm-header">Permission</span>
          <span class="overwrite-perm-header">Allow</span>
          <span class="overwrite-perm-header">Deny</span>
          <For each={PERMISSION_CATEGORIES}>
            {(category) => (
              <For each={category.permissions}>
                {(perm) => (
                  <>
                    <span>{perm.label}</span>
                    <input
                      type="checkbox"
                      class="role-picker-checkbox"
                      checked={hasPerm(overwriteAllow(), perm.value)}
                      onChange={() => {
                        setOverwriteAllow(togglePermBit(overwriteAllow(), perm.value));
                        if (hasPerm(overwriteDeny(), perm.value)) {
                          setOverwriteDeny(togglePermBit(overwriteDeny(), perm.value));
                        }
                      }}
                    />
                    <input
                      type="checkbox"
                      class="role-picker-checkbox"
                      checked={hasPerm(overwriteDeny(), perm.value)}
                      onChange={() => {
                        setOverwriteDeny(togglePermBit(overwriteDeny(), perm.value));
                        if (hasPerm(overwriteAllow(), perm.value)) {
                          setOverwriteAllow(togglePermBit(overwriteAllow(), perm.value));
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
          <button type="button" class="form-btn-primary" onClick={handleSaveOverwrite}>
            <SkinGlyph name="save" /> Save Overwrite
          </button>
          <button type="button" class="form-btn-danger" onClick={handleDeleteOverwrite}>
            <SkinGlyph name="delete" /> Remove Overwrite
          </button>
        </div>
      </Show>
    </div>
  );
};

export default ChannelOverwriteEditor;
