import { Component, For, Show, createSignal } from "solid-js";
import type { Community } from "../../../stores/community.store";
import type { ConfirmOptions } from "../CommunitySettingsModal";
import RoleTag from "../RoleTag";
import PermissionCheckboxList from "./PermissionCheckboxList";
import FormField from "../../common/FormField";
import { colorIntToHex, hexToColorInt } from "../../../utils/color";
import { togglePermBit } from "../../../ipc/permissions";
import {
  handleCreateRole,
  handleEditRole,
  handleDeleteRole,
} from "../../../handlers/community.handlers";
import { addToast } from "../../../stores/toast.store";
import {
  ICON_SAVE,
  ICON_DELETE,
  ICON_CLOSE,
  ICON_PLUS_BOX,
} from "../../../icons";

interface RolesTabProps {
  community: Community;
  requestConfirm: (opts: ConfirmOptions) => void;
}

const RolesTab: Component<RolesTabProps> = (props) => {
  const [showNewRole, setShowNewRole] = createSignal(false);
  const [creatingRole, setCreatingRole] = createSignal(false);
  const [newRoleName, setNewRoleName] = createSignal("");
  const [newRoleColor, setNewRoleColor] = createSignal("#000000");
  const [newRolePerms, setNewRolePerms] = createSignal(0n);
  const [newRoleHoist, setNewRoleHoist] = createSignal(false);
  const [newRoleMentionable, setNewRoleMentionable] = createSignal(false);
  const [editingRoleId, setEditingRoleId] = createSignal<number | null>(null);
  const [editRoleName, setEditRoleName] = createSignal("");
  const [editRoleColor, setEditRoleColor] = createSignal("#000000");
  const [editRolePerms, setEditRolePerms] = createSignal(0n);
  const [editRoleHoist, setEditRoleHoist] = createSignal(false);
  const [editRoleMentionable, setEditRoleMentionable] = createSignal(false);
  const [savingRole, setSavingRole] = createSignal(false);

  function startEditRole(role: { id: number; name: string; color: number; permissions: string; hoist: boolean; mentionable: boolean }): void {
    setEditingRoleId(role.id);
    setEditRoleName(role.name);
    setEditRoleColor(colorIntToHex(role.color));
    setEditRolePerms(BigInt(role.permissions));
    setEditRoleHoist(role.hoist);
    setEditRoleMentionable(role.mentionable);
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
      );
      setNewRoleName("");
      setNewRoleColor("#000000");
      setNewRolePerms(0n);
      setNewRoleHoist(false);
      setNewRoleMentionable(false);
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

  function confirmDeleteRole(roleId: number, roleName: string): void {
    props.requestConfirm({
      title: "Delete Role",
      message: `Delete the "${roleName}" role? This cannot be undone.`,
      confirmLabel: "Delete",
      action: () => {
        handleDeleteRole(props.community.id, roleId);
        if (editingRoleId() === roleId) setEditingRoleId(null);
      },
    });
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
              onClick={() => role.id > 0 && startEditRole(role)}
            >
              <span
                class="settings-role-color"
                style={{ background: role.color ? colorIntToHex(role.color) : "var(--color-xfire-text-dim)" }}
              />
              <RoleTag name={role.name} color={role.color} />
              <span class="settings-role-position">pos: {role.position}</span>
              <Show when={role.hoist}>
                <span class="settings-role-position">hoisted</span>
              </Show>
              <div class="settings-role-actions">
                <Show when={role.id > 1}>
                  <button
                    class="form-btn-danger"
                    onClick={(e) => { e.stopPropagation(); confirmDeleteRole(role.id, role.name); }}
                    title="Delete Role"
                  >
                    <span class="nf-icon">{ICON_DELETE}</span>
                  </button>
                </Show>
              </div>
            </div>
          }>
            <div class="settings-section" style={{ "padding-left": "8px", "border-left": "2px solid var(--color-xfire-accent)" }}>
              <FormField label="Name">
                <input
                  class="form-input"
                  type="text"
                  value={editRoleName()}
                  onInput={(e) => setEditRoleName(e.currentTarget.value)}
                />
              </FormField>
              <FormField label="Color">
                <div class="form-field-row">
                  <input
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
              </FormField>
              <FormField label="Permissions">
                <PermissionCheckboxList
                  permissions={editRolePerms()}
                  onToggle={(bit) => setEditRolePerms(togglePermBit(editRolePerms(), bit))}
                />
              </FormField>
              <div class="form-field-row">
                <button class="form-btn-save" onClick={handleSaveEditRole} disabled={savingRole()}>
                  <span class="nf-icon">{ICON_SAVE}</span> {savingRole() ? "Saving..." : "Save"}
                </button>
                <button class="form-btn-secondary" onClick={() => setEditingRoleId(null)} disabled={savingRole()}>
                  <span class="nf-icon">{ICON_CLOSE}</span> Cancel
                </button>
              </div>
            </div>
          </Show>
        )}
      </For>
      <Show when={showNewRole()} fallback={
        <button
          class="form-btn-secondary"
          onClick={() => setShowNewRole(true)}
        >
          <span class="nf-icon">{ICON_PLUS_BOX}</span> Create Role
        </button>
      }>
        <div class="settings-section" style={{ "padding-left": "8px", "border-left": "2px solid var(--color-xfire-online)" }}>
          <FormField label="Name">
            <input
              class="form-input"
              type="text"
              placeholder="Role name..."
              value={newRoleName()}
              onInput={(e) => setNewRoleName(e.currentTarget.value)}
            />
          </FormField>
          <FormField label="Color">
            <div class="form-field-row">
              <input
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
          </FormField>
          <FormField label="Permissions">
            <PermissionCheckboxList
              permissions={newRolePerms()}
              onToggle={(bit) => setNewRolePerms(togglePermBit(newRolePerms(), bit))}
            />
          </FormField>
          <div class="form-field-row">
            <button
              class="form-btn-save"
              onClick={handleCreateNewRole}
              disabled={!newRoleName().trim() || creatingRole()}
            >
              <span class="nf-icon">{ICON_SAVE}</span> {creatingRole() ? "Creating..." : "Create"}
            </button>
            <button
              class="form-btn-secondary"
              onClick={() => setShowNewRole(false)}
            >
              <span class="nf-icon">{ICON_CLOSE}</span> Cancel
            </button>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default RolesTab;
