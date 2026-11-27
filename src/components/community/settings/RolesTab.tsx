import { type Component, For, Show, createSignal, createUniqueId } from "solid-js";
import SkinGlyph from "../../common/SkinGlyph";
import type { Community } from "../../../stores/community.store";
import { EVERYONE_ROLE_ID } from "../../../stores/community.store";
import RoleTag from "../RoleTag";
import PermissionCheckboxList from "./PermissionCheckboxList";
import FormField from "../../common/FormField";
import { colorIntToHex, hexToColorInt } from "../../../utils/color";
import { togglePermBit } from "../../../ipc/permissions";
import {
  handleCreateRole,
  handleEditRole,
  handleDeleteRole,
} from "../../../actions/community.actions";
import { addToast } from "../../../stores/toast.store";
import { tokenVar } from "../../../styles/generated/tokens";

interface RolesTabProps {
  community: Community;
}

const RolesTab: Component<RolesTabProps> = (props) => {
  const editRoleNameId = createUniqueId();
  const editRoleColorId = createUniqueId();
  const newRoleNameId = createUniqueId();
  const newRoleColorId = createUniqueId();
  const [showNewRole, setShowNewRole] = createSignal(false);
  const [creatingRole, setCreatingRole] = createSignal(false);
  const [newRoleName, setNewRoleName] = createSignal("");
  const [newRoleColor, setNewRoleColor] = createSignal("#000000");
  const [newRolePerms, setNewRolePerms] = createSignal(0n);
  const [newRoleHoist, setNewRoleHoist] = createSignal(false);
  const [newRoleMentionable, setNewRoleMentionable] = createSignal(false);
  const [newRoleSelfAssignable, setNewRoleSelfAssignable] = createSignal(false);
  const [editingRoleId, setEditingRoleId] = createSignal<string | null>(null);
  const [editRoleName, setEditRoleName] = createSignal("");
  const [editRoleColor, setEditRoleColor] = createSignal("#000000");
  const [editRolePerms, setEditRolePerms] = createSignal(0n);
  const [editRoleHoist, setEditRoleHoist] = createSignal(false);
  const [editRoleMentionable, setEditRoleMentionable] = createSignal(false);
  const [editRoleSelfAssignable, setEditRoleSelfAssignable] = createSignal(false);
  const [savingRole, setSavingRole] = createSignal(false);

  function startEditRole(role: { id: string; name: string; color: number; permissions: string; hoist: boolean; mentionable: boolean; selfAssignable?: boolean }): void {
    setEditingRoleId(role.id);
    setEditRoleName(role.name);
    setEditRoleColor(colorIntToHex(role.color));
    setEditRolePerms(BigInt(role.permissions));
    setEditRoleHoist(role.hoist);
    setEditRoleMentionable(role.mentionable);
    setEditRoleSelfAssignable(role.selfAssignable ?? false);
  }

  async function handleCreateNewRole(): Promise<void> {
    const name = newRoleName().trim();
    if (!name) return;
    setCreatingRole(true);
    try {
      await handleCreateRole(
        props.community.id,
        name,
        hexToColorInt(newRoleColor()),
        newRolePerms().toString(),
        newRoleHoist(),
        newRoleMentionable(),
        newRoleSelfAssignable(),
      );
      setNewRoleName("");
      setNewRoleColor("#000000");
      setNewRolePerms(0n);
      setNewRoleHoist(false);
      setNewRoleMentionable(false);
      setNewRoleSelfAssignable(false);
      setShowNewRole(false);
    } finally {
      setCreatingRole(false);
    }
  }

  async function handleSaveEditRole(): Promise<void> {
    const id = editingRoleId();
    if (id === null) return;
    setSavingRole(true);
    try {
      await handleEditRole(
        props.community.id,
        id,
        editRoleName().trim() || null,
        hexToColorInt(editRoleColor()),
        editRolePerms().toString(),
        null,
        editRoleHoist(),
        editRoleMentionable(),
        editRoleSelfAssignable(),
      );
      addToast("Role updated", "success");
      setEditingRoleId(null);
    } catch (e) {
      const msg = typeof e === "string" ? e : "Failed to save role";
      addToast(msg, "error");
    } finally {
      setSavingRole(false);
    }
  }

  function deleteRole(roleId: string): void {
    void handleDeleteRole(props.community.id, roleId);
    if (editingRoleId() === roleId) setEditingRoleId(null);
  }

  return (
    <div class="settings-section">
      <div class="settings-hint">
        Roles are ordered by position. Higher position = more authority. Click a role to edit it.
      </div>
      <For each={[...props.community.roles].sort((a, b) => b.position - a.position)}>
        {(role) => (
          <Show when={editingRoleId() === role.id} fallback={
            <div
              class="settings-role-row"
              onClick={() => role.id !== EVERYONE_ROLE_ID && startEditRole(role)}
            >
              <span
                class="settings-role-color"
                style={{ "--settings-role-color": role.color ? colorIntToHex(role.color) : tokenVar("text.dim") }}
              />
              <RoleTag name={role.name} color={role.color} tagColours={role.tagColours} />
              <span class="settings-role-position">pos: {role.position}</span>
              <Show when={role.hoist}>
                <span class="settings-role-position">hoisted</span>
              </Show>
              <div class="settings-role-actions">
                <Show when={role.id !== EVERYONE_ROLE_ID}>
                  <button
                    type="button"
                    class="form-btn-danger"
                    onClick={(e) => { e.stopPropagation(); deleteRole(role.id); }}
                    title="Delete Role"
                  >
                    <SkinGlyph name="delete" />
                  </button>
                </Show>
              </div>
            </div>
          }>
            <div class="settings-section settings-section-accented">
              <FormField label="Name" controlId={editRoleNameId}>
                <input
                  id={editRoleNameId}
                  class="form-input"
                  type="text"
                  value={editRoleName()}
                  onInput={(e) => setEditRoleName(e.currentTarget.value)}
                />
              </FormField>
              <FormField label="Color" controlId={editRoleColorId}>
                <div class="form-field-row">
                  <input
                    id={editRoleColorId}
                    type="color"
                    value={editRoleColor()}
                    onInput={(e) => setEditRoleColor(e.currentTarget.value)}
                  />
                  <span class="settings-value">{editRoleColor()}</span>
                </div>
              </FormField>
              <FormField label="Options">
                <div class="form-field-row">
                  <label class="settings-option">
                    <input
                      type="checkbox"
                      checked={editRoleHoist()}
                      onChange={(e) => setEditRoleHoist(e.currentTarget.checked)}
                    />
                    Hoist (show separately in member list)
                  </label>
                </div>
                <div class="form-field-row">
                  <label class="settings-option">
                    <input
                      type="checkbox"
                      checked={editRoleMentionable()}
                      onChange={(e) => setEditRoleMentionable(e.currentTarget.checked)}
                    />
                    Mentionable
                  </label>
                </div>
                <div class="form-field-row">
                  <label class="settings-option">
                    <input
                      type="checkbox"
                      checked={editRoleSelfAssignable()}
                      onChange={(e) => setEditRoleSelfAssignable(e.currentTarget.checked)}
                    />
                    Self-assignable
                  </label>
                </div>
              </FormField>
              <FormField label="Permissions">
                <PermissionCheckboxList
                  permissions={editRolePerms()}
                  onToggle={(bit) => setEditRolePerms(togglePermBit(editRolePerms(), bit))}
                />
              </FormField>
              <div class="form-field-row">
                <button type="button" class="form-btn-primary" onClick={handleSaveEditRole} disabled={savingRole()}>
                  <SkinGlyph name="save" /> {savingRole() ? "Saving..." : "Save"}
                </button>
                <button type="button" class="form-btn-secondary" onClick={() => setEditingRoleId(null)} disabled={savingRole()}>
                  <SkinGlyph name="close" /> Cancel
                </button>
              </div>
            </div>
          </Show>
        )}
      </For>
      <Show when={showNewRole()} fallback={
        <button
          type="button"
          class="form-btn-secondary"
          onClick={() => setShowNewRole(true)}
        >
          <SkinGlyph name="plus-box" /> Create Role
        </button>
      }>
        <div class="settings-section settings-section-success">
          <FormField label="Name" controlId={newRoleNameId}>
            <input
              id={newRoleNameId}
              class="form-input"
              type="text"
              placeholder="Role name..."
              value={newRoleName()}
              onInput={(e) => setNewRoleName(e.currentTarget.value)}
            />
          </FormField>
          <FormField label="Color" controlId={newRoleColorId}>
            <div class="form-field-row">
              <input
                id={newRoleColorId}
                type="color"
                value={newRoleColor()}
                onInput={(e) => setNewRoleColor(e.currentTarget.value)}
              />
              <span class="settings-value">{newRoleColor()}</span>
            </div>
          </FormField>
          <FormField label="Options">
            <div class="form-field-row">
              <label class="settings-option">
                <input
                  type="checkbox"
                  checked={newRoleHoist()}
                  onChange={(e) => setNewRoleHoist(e.currentTarget.checked)}
                />
                Hoist (show separately)
              </label>
            </div>
            <div class="form-field-row">
              <label class="settings-option">
                <input
                  type="checkbox"
                  checked={newRoleMentionable()}
                  onChange={(e) => setNewRoleMentionable(e.currentTarget.checked)}
                />
                Mentionable
              </label>
            </div>
            <div class="form-field-row">
              <label class="settings-option">
                <input
                  type="checkbox"
                  checked={newRoleSelfAssignable()}
                  onChange={(e) => setNewRoleSelfAssignable(e.currentTarget.checked)}
                />
                Self-assignable
              </label>
            </div>
          </FormField>
          <FormField label="Permissions">
            <PermissionCheckboxList
              permissions={newRolePerms()}
              onToggle={(bit) => setNewRolePerms(togglePermBit(newRolePerms(), bit))}
            />
          </FormField>
          <div class="form-field-row">
            <button
              type="button"
              class="form-btn-primary"
              onClick={handleCreateNewRole}
              disabled={!newRoleName().trim() || creatingRole()}
            >
              <SkinGlyph name="save" /> {creatingRole() ? "Creating..." : "Create"}
            </button>
            <button
              type="button"
              class="form-btn-secondary"
              onClick={() => setShowNewRole(false)}
            >
              <SkinGlyph name="close" /> Cancel
            </button>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default RolesTab;
