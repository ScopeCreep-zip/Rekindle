import { Component, For, Show, createSignal, createEffect, createMemo } from "solid-js";
import Modal from "../common/Modal";
import ConfirmDialog from "../common/ConfirmDialog";
import StatusDot from "../status/StatusDot";
import RoleTag from "./RoleTag";
import { commands } from "../../ipc/commands";
import type { Community, Member, Role } from "../../stores/community.store";
import {
  handleDeleteChannel,
  handleRenameChannel,
  handleCreateChannel,
  handleUpdateCommunityInfo,
  handleRemoveCommunityMember,
  handleBanMember,
  handleUnbanMember,
  handleGetBanList,
  handleRotateMek,
  handleAssignRole,
  handleUnassignRole,
  handleTimeoutMember,
  handleRemoveTimeout,
  handleCreateRole,
  handleEditRole,
  handleDeleteRole,
} from "../../handlers/community.handlers";
import {
  calculateBasePermissions,
  highestPosition,
  hasPermission,
  MANAGE_CHANNELS,
  MANAGE_COMMUNITY,
  MANAGE_ROLES,
  KICK_MEMBERS,
  BAN_MEMBERS,
  MODERATE_MEMBERS,
  PERMISSION_CATEGORIES,
} from "../../ipc/permissions";
import { addToast } from "../../stores/toast.store";
import {
  ICON_SAVE,
  ICON_PENCIL,
  ICON_DELETE,
  ICON_PERMS,
  ICON_PLUS,
  ICON_PLUS_BOX,
  ICON_CLOSE,
  ICON_COPY,
  ICON_CHANNEL_TEXT,
  ICON_VOLUME_HIGH,
  ICON_SHIELD,
  ICON_TIMEOUT,
  ICON_ACCOUNT_REMOVE,
  ICON_BAN,
  ICON_REFRESH,
  ICON_KEY,
} from "../../icons";

interface CommunitySettingsModalProps {
  isOpen: boolean;
  community: Community;
  myRoleIds: number[];
  onClose: () => void;
}

type TabId = "overview" | "channels" | "members" | "roles" | "bans" | "security";

const CommunitySettingsModal: Component<CommunitySettingsModalProps> = (props) => {
  const [activeTab, setActiveTab] = createSignal<TabId>("overview");

  // Overview tab state
  const [editName, setEditName] = createSignal("");
  const [editDescription, setEditDescription] = createSignal("");
  const [copied, setCopied] = createSignal(false);
  const [savingOverview, setSavingOverview] = createSignal(false);

  // Channels tab state
  const [renamingChannelId, setRenamingChannelId] = createSignal<string | null>(null);
  const [renameValue, setRenameValue] = createSignal("");
  const [showNewChannel, setShowNewChannel] = createSignal(false);
  const [newChannelName, setNewChannelName] = createSignal("");
  const [newChannelType, setNewChannelType] = createSignal<"text" | "voice">("text");
  const [creatingChannel, setCreatingChannel] = createSignal(false);

  // Channel overwrite editor state
  const [overwriteChannelId, setOverwriteChannelId] = createSignal<string | null>(null);
  const [overwriteTargetType, setOverwriteTargetType] = createSignal("role");
  const [overwriteTargetId, setOverwriteTargetId] = createSignal("");
  const [overwriteAllow, setOverwriteAllow] = createSignal(0);
  const [overwriteDeny, setOverwriteDeny] = createSignal(0);

  // Members tab state
  const [rolePickerTarget, setRolePickerTarget] = createSignal<string | null>(null);

  // Bans tab state
  const [banList, setBanList] = createSignal<{ pseudonymKey: string; displayName: string; bannedAt: number }[]>([]);
  const [bansLoaded, setBansLoaded] = createSignal(false);

  // Roles tab state
  const [newRoleName, setNewRoleName] = createSignal("");
  const [newRoleColor, setNewRoleColor] = createSignal("#000000");
  const [newRolePerms, setNewRolePerms] = createSignal(0);
  const [newRoleHoist, setNewRoleHoist] = createSignal(false);
  const [newRoleMentionable, setNewRoleMentionable] = createSignal(false);
  const [showNewRole, setShowNewRole] = createSignal(false);
  const [creatingRole, setCreatingRole] = createSignal(false);
  const [editingRoleId, setEditingRoleId] = createSignal<number | null>(null);
  const [editRoleName, setEditRoleName] = createSignal("");
  const [editRoleColor, setEditRoleColor] = createSignal("#000000");
  const [editRolePerms, setEditRolePerms] = createSignal(0);
  const [editRoleHoist, setEditRoleHoist] = createSignal(false);
  const [editRoleMentionable, setEditRoleMentionable] = createSignal(false);

  // Confirm dialog state
  const [confirmAction, setConfirmAction] = createSignal<{
    title: string; message: string; action: () => void; confirmLabel?: string;
  } | null>(null);

  const myPerms = createMemo(() =>
    calculateBasePermissions(props.myRoleIds, props.community.roles, props.community.isHosted),
  );

  const canManageCommunity = createMemo(() => hasPermission(myPerms(), MANAGE_COMMUNITY));
  const canManageChannels = createMemo(() => hasPermission(myPerms(), MANAGE_CHANNELS));
  const canManageRoles = createMemo(() => hasPermission(myPerms(), MANAGE_ROLES));
  const canKick = createMemo(() => hasPermission(myPerms(), KICK_MEMBERS));
  const canBan = createMemo(() => hasPermission(myPerms(), BAN_MEMBERS));
  const canModerate = createMemo(() => hasPermission(myPerms(), MODERATE_MEMBERS));

  const tabs = createMemo((): { id: TabId; label: string }[] => {
    const base: { id: TabId; label: string }[] = [
      { id: "overview", label: "Overview" },
      { id: "channels", label: "Channels" },
      { id: "members", label: "Members" },
    ];
    if (canManageRoles()) {
      base.push({ id: "roles", label: "Roles" });
    }
    if (canBan()) {
      base.push({ id: "bans", label: "Bans" });
    }
    if (canManageCommunity()) {
      base.push({ id: "security", label: "Security" });
    }
    return base;
  });

  createEffect(() => {
    if (props.isOpen) {
      setEditName(props.community.name);
      setEditDescription(props.community.description ?? "");
      setCopied(false);
      setRenamingChannelId(null);
      setShowNewChannel(false);
      setRolePickerTarget(null);
      setBansLoaded(false);
      setShowNewRole(false);
      setNewRoleName("");
      setActiveTab("overview");
      setOverwriteChannelId(null);
      setConfirmAction(null);
    }
  });

  createEffect(() => {
    if (activeTab() === "bans" && !bansLoaded() && canBan()) {
      setBansLoaded(true);
      handleGetBanList(props.community.id).then(setBanList);
    }
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

  function startRename(channel: { id: string; name: string }): void {
    setRenamingChannelId(channel.id);
    setRenameValue(channel.name);
  }

  async function submitRename(channelId: string): Promise<void> {
    const val = renameValue().trim();
    if (val) {
      await handleRenameChannel(props.community.id, channelId, val);
    }
    setRenamingChannelId(null);
  }

  function confirmDeleteChannel(channel: { id: string; name: string }): void {
    setConfirmAction({
      title: "Delete Channel",
      message: `Delete #${channel.name}? This cannot be undone.`,
      confirmLabel: "Delete",
      action: () => handleDeleteChannel(props.community.id, channel.id),
    });
  }

  async function handleCreateCh(): Promise<void> {
    const n = newChannelName().trim();
    if (!n) return;
    setCreatingChannel(true);
    try {
      await handleCreateChannel(props.community.id, n, newChannelType());
      setNewChannelName("");
      setNewChannelType("text");
      setShowNewChannel(false);
    } finally {
      setCreatingChannel(false);
    }
  }

  function confirmKick(member: Member): void {
    setConfirmAction({
      title: "Kick Member",
      message: `Kick ${member.displayName} from the community?`,
      confirmLabel: "Kick",
      action: () => handleRemoveCommunityMember(props.community.id, member.pseudonymKey),
    });
  }

  function confirmBan(member: Member): void {
    setConfirmAction({
      title: "Ban Member",
      message: `Ban ${member.displayName}? They will not be able to rejoin.`,
      confirmLabel: "Ban",
      action: () => handleBanMember(props.community.id, member.pseudonymKey),
    });
  }

  async function handleUnban(pseudonymKey: string): Promise<void> {
    await handleUnbanMember(props.community.id, pseudonymKey);
    setBanList((prev) => prev.filter((b) => b.pseudonymKey !== pseudonymKey));
  }

  function toggleMemberRole(pseudonymKey: string, roleId: number, has: boolean): void {
    if (has) {
      handleUnassignRole(props.community.id, pseudonymKey, roleId);
    } else {
      handleAssignRole(props.community.id, pseudonymKey, roleId);
    }
  }

  function confirmRotateKey(): void {
    setConfirmAction({
      title: "Rotate Encryption Key",
      message: "Generate a new Media Encryption Key? All members will automatically receive the new key.",
      confirmLabel: "Rotate",
      action: () => handleRotateMek(props.community.id),
    });
  }

  function memberAllRoles(member: Member): { name: string; color: number }[] {
    return member.roleIds
      .map((id) => props.community.roles.find((r) => r.id === id))
      .filter((r): r is Role => r != null && r.name !== "@everyone")
      .sort((a, b) => b.position - a.position)
      .map((r) => ({ name: r.name, color: r.color }));
  }

  function canManageMember(member: Member): boolean {
    const myPos = highestPosition(props.myRoleIds, props.community.roles);
    const memberPos = highestPosition(member.roleIds, props.community.roles);
    return myPos > memberPos && member.pseudonymKey !== props.community.myPseudonymKey;
  }

  function hexToInt(hex: string): number {
    return parseInt(hex.replace("#", ""), 16);
  }

  function intToHex(n: number): string {
    if (!n) return "#000000";
    return `#${(n & 0xFFFFFF).toString(16).padStart(6, "0")}`;
  }

  function togglePermBit(current: number, bit: number): number {
    if (bit > 0x7FFF_FFFF) {
      const hasBit = Math.floor(current / bit) % 2 === 1;
      return hasBit ? current - bit : current + bit;
    }
    return current ^ bit;
  }

  function hasPerm(perms: number, bit: number): boolean {
    if (bit > 0x7FFF_FFFF) {
      return Math.floor(perms / bit) % 2 === 1;
    }
    return (perms & bit) !== 0;
  }

  async function handleCreateNewRole(): Promise<void> {
    const name = newRoleName().trim();
    if (!name) return;
    setCreatingRole(true);
    try {
      await handleCreateRole(
        props.community.id,
        name,
        hexToInt(newRoleColor()),
        newRolePerms(),
        newRoleHoist(),
        newRoleMentionable(),
      );
      setNewRoleName("");
      setNewRoleColor("#000000");
      setNewRolePerms(0);
      setNewRoleHoist(false);
      setNewRoleMentionable(false);
      setShowNewRole(false);
    } finally {
      setCreatingRole(false);
    }
  }

  function startEditRole(role: { id: number; name: string; color: number; permissions: number; hoist: boolean; mentionable: boolean }): void {
    setEditingRoleId(role.id);
    setEditRoleName(role.name);
    setEditRoleColor(intToHex(role.color));
    setEditRolePerms(role.permissions);
    setEditRoleHoist(role.hoist);
    setEditRoleMentionable(role.mentionable);
  }

  async function handleSaveEditRole(): Promise<void> {
    const id = editingRoleId();
    if (id === null) return;
    await handleEditRole(
      props.community.id,
      id,
      editRoleName().trim() || null,
      hexToInt(editRoleColor()),
      editRolePerms(),
      null,
      editRoleHoist(),
      editRoleMentionable(),
    );
    setEditingRoleId(null);
  }

  function confirmDeleteRole(roleId: number, roleName: string): void {
    setConfirmAction({
      title: "Delete Role",
      message: `Delete the "${roleName}" role? This cannot be undone.`,
      confirmLabel: "Delete",
      action: () => {
        handleDeleteRole(props.community.id, roleId);
        if (editingRoleId() === roleId) setEditingRoleId(null);
      },
    });
  }

  async function handleSaveOverwrite(): Promise<void> {
    const channelId = overwriteChannelId();
    const targetId = overwriteTargetId();
    if (!channelId || !targetId) return;
    try {
      await commands.setChannelOverwrite(
        props.community.id,
        channelId,
        overwriteTargetType(),
        targetId,
        overwriteAllow(),
        overwriteDeny(),
      );
      addToast("Permission overwrite saved", "success");
    } catch (e) {
      console.error("Failed to save overwrite:", e);
      addToast("Failed to save overwrite", "error");
    }
  }

  async function handleDeleteOverwrite(): Promise<void> {
    const channelId = overwriteChannelId();
    const targetId = overwriteTargetId();
    if (!channelId || !targetId) return;
    try {
      await commands.deleteChannelOverwrite(
        props.community.id,
        channelId,
        overwriteTargetType(),
        targetId,
      );
      setOverwriteAllow(0);
      setOverwriteDeny(0);
      addToast("Permission overwrite removed", "success");
    } catch (e) {
      console.error("Failed to delete overwrite:", e);
      addToast("Failed to delete overwrite", "error");
    }
  }

  return (
    <Modal isOpen={props.isOpen} title="Community Settings" onClose={props.onClose} size="lg">
      <div class="settings-tabs">
        <For each={tabs()}>
          {(tab) => (
            <button
              class={`settings-tab ${activeTab() === tab.id ? "settings-tab-active" : ""}`}
              onClick={() => setActiveTab(tab.id)}
            >
              {tab.label}
            </button>
          )}
        </For>
      </div>

      <div class="settings-content">
        {/* Overview Tab */}
        <Show when={activeTab() === "overview"}>
          <div class="settings-section">
            <div class="settings-field">
              <label class="settings-field-label">Community Name</label>
              <Show when={canManageCommunity()} fallback={
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
              <Show when={canManageCommunity()} fallback={
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
            <Show when={canManageCommunity()}>
              <div class="settings-field">
                <button class="settings-save-btn" onClick={handleSaveOverview} disabled={savingOverview()}>
                  <span class="nf-icon">{ICON_SAVE}</span> {savingOverview() ? "Saving..." : "Save Changes"}
                </button>
              </div>
            </Show>
          </div>
        </Show>

        {/* Channels Tab */}
        <Show when={activeTab() === "channels"}>
          <div class="settings-section">
            <For each={props.community.channels}>
              {(channel) => (
                <div>
                  <div class="channel-manage-row">
                    <span class="nf-icon channel-manage-icon">
                      {channel.type === "voice" ? ICON_VOLUME_HIGH : ICON_CHANNEL_TEXT}
                    </span>
                    <Show when={renamingChannelId() === channel.id} fallback={
                      <span class="channel-manage-name">{channel.name}</span>
                    }>
                      <input
                        class="settings-input channel-rename-input"
                        type="text"
                        value={renameValue()}
                        onInput={(e) => setRenameValue(e.currentTarget.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") submitRename(channel.id);
                          if (e.key === "Escape") setRenamingChannelId(null);
                        }}
                      />
                    </Show>
                    <span class="channel-manage-type">{channel.type}</span>
                    <Show when={canManageChannels()}>
                      <Show when={renamingChannelId() !== channel.id}>
                        <button
                          class="settings-action-btn channel-manage-btn"
                          onClick={() => startRename(channel)}
                          title="Rename"
                        >
                          <span class="nf-icon">{ICON_PENCIL}</span>
                        </button>
                      </Show>
                      <Show when={renamingChannelId() === channel.id}>
                        <button
                          class="settings-save-btn channel-manage-btn"
                          onClick={() => submitRename(channel.id)}
                          title="Save"
                        >
                          <span class="nf-icon">{ICON_SAVE}</span>
                        </button>
                      </Show>
                      <button
                        class="settings-danger-btn channel-manage-btn"
                        onClick={() => confirmDeleteChannel(channel)}
                        title="Delete"
                      >
                        <span class="nf-icon">{ICON_DELETE}</span>
                      </button>
                      <button
                        class="settings-action-btn channel-manage-btn"
                        onClick={() => {
                          const next = overwriteChannelId() === channel.id ? null : channel.id;
                          setOverwriteChannelId(next);
                          setOverwriteTargetId("");
                          setOverwriteAllow(0);
                          setOverwriteDeny(0);
                        }}
                        title="Permissions"
                      >
                        <span class="nf-icon">{ICON_PERMS}</span>
                      </button>
                    </Show>
                  </div>
                  <Show when={overwriteChannelId() === channel.id && canManageChannels()}>
                    <div class="overwrite-editor">
                      <div class="settings-field-row">
                        <select
                          class="settings-select"
                          value={overwriteTargetType()}
                          onChange={(e) => {
                            setOverwriteTargetType(e.currentTarget.value);
                            setOverwriteTargetId("");
                          }}
                        >
                          <option value="role">Role</option>
                        </select>
                        <select
                          class="settings-select"
                          value={overwriteTargetId()}
                          onChange={(e) => setOverwriteTargetId(e.currentTarget.value)}
                        >
                          <option value="">Select target...</option>
                          <For each={props.community.roles}>
                            {(role) => (
                              <option value={String(role.id)}>{role.name}</option>
                            )}
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
                        <div class="settings-field-row">
                          <button class="settings-save-btn" onClick={handleSaveOverwrite}>
                            <span class="nf-icon">{ICON_SAVE}</span> Save Overwrite
                          </button>
                          <button class="settings-danger-btn" onClick={handleDeleteOverwrite}>
                            <span class="nf-icon">{ICON_DELETE}</span> Remove Overwrite
                          </button>
                        </div>
                      </Show>
                    </div>
                  </Show>
                </div>
              )}
            </For>
            <Show when={canManageChannels()}>
              <Show when={showNewChannel()} fallback={
                <button
                  class="settings-action-btn"
                  onClick={() => setShowNewChannel(true)}
                >
                  <span class="nf-icon">{ICON_PLUS_BOX}</span> Create Channel
                </button>
              }>
                <div class="channel-create-inline">
                  <input
                    class="settings-input"
                    type="text"
                    placeholder="Channel name..."
                    value={newChannelName()}
                    onInput={(e) => setNewChannelName(e.currentTarget.value)}
                  />
                  <select
                    class="settings-select channel-type-select"
                    value={newChannelType()}
                    onChange={(e) => setNewChannelType(e.currentTarget.value as "text" | "voice")}
                  >
                    <option value="text">Text</option>
                    <option value="voice">Voice</option>
                  </select>
                  <button
                    class="settings-save-btn"
                    onClick={handleCreateCh}
                    disabled={!newChannelName().trim() || creatingChannel()}
                  >
                    {creatingChannel() ? "Creating..." : "Create"}
                  </button>
                  <button
                    class="settings-action-btn"
                    onClick={() => setShowNewChannel(false)}
                  >
                    Cancel
                  </button>
                </div>
              </Show>
            </Show>
          </div>
        </Show>

        {/* Members Tab */}
        <Show when={activeTab() === "members"}>
          <div class="settings-section">
            <div class="member-list-header">
              Members — {props.community.members.length}
            </div>
            <For each={props.community.members}>
              {(member) => (
                <div class="settings-member-row">
                  <StatusDot status={member.status || "online"} />
                  <div class="member-name-info">
                    <span class="member-name">{member.displayName}</span>
                    <div class="member-roles-row">
                      <For each={memberAllRoles(member)}>
                        {(role) => <RoleTag name={role.name} color={role.color} />}
                      </For>
                    </div>
                    <Show when={member.timeoutUntil}>
                      <span class="nf-icon timeout-indicator" title="Timed out">{ICON_TIMEOUT}</span>
                    </Show>
                  </div>
                  <Show when={canManageMember(member)}>
                    <div class="settings-member-actions">
                      <Show when={canManageRoles()}>
                        <button
                          class="settings-action-btn"
                          onClick={() => setRolePickerTarget(
                            rolePickerTarget() === member.pseudonymKey ? null : member.pseudonymKey
                          )}
                        >
                          <span class="nf-icon">{ICON_SHIELD}</span> Roles
                        </button>
                      </Show>
                      <Show when={canModerate()}>
                        <Show when={!member.timeoutUntil} fallback={
                          <button
                            class="settings-action-btn"
                            onClick={() => handleRemoveTimeout(props.community.id, member.pseudonymKey)}
                          >
                            <span class="nf-icon">{ICON_TIMEOUT}</span> Untimeout
                          </button>
                        }>
                          <select
                            class="timeout-select"
                            onChange={(e) => {
                              const secs = Number(e.currentTarget.value);
                              if (secs > 0) {
                                handleTimeoutMember(props.community.id, member.pseudonymKey, secs, null);
                                e.currentTarget.value = "";
                              }
                            }}
                          >
                            <option value="">Timeout...</option>
                            <option value="60">1 min</option>
                            <option value="300">5 min</option>
                            <option value="600">10 min</option>
                            <option value="1800">30 min</option>
                            <option value="3600">1 hour</option>
                            <option value="86400">1 day</option>
                          </select>
                        </Show>
                      </Show>
                      <Show when={canKick()}>
                        <button
                          class="settings-action-btn"
                          onClick={() => confirmKick(member)}
                        >
                          <span class="nf-icon">{ICON_ACCOUNT_REMOVE}</span> Kick
                        </button>
                      </Show>
                      <Show when={canBan()}>
                        <button
                          class="settings-danger-btn"
                          onClick={() => confirmBan(member)}
                        >
                          <span class="nf-icon">{ICON_BAN}</span> Ban
                        </button>
                      </Show>
                    </div>
                  </Show>
                  <Show when={rolePickerTarget() === member.pseudonymKey}>
                    <div class="role-picker-list settings-role-picker">
                      <For each={props.community.roles.filter((r) => r.id !== 0).sort((a, b) => b.position - a.position)}>
                        {(role) => {
                          const has = () => member.roleIds.includes(role.id);
                          return (
                            <div
                              class={`role-picker-item ${has() ? "role-picker-item-active" : ""}`}
                              onClick={() => toggleMemberRole(member.pseudonymKey, role.id, has())}
                            >
                              <input type="checkbox" class="role-picker-checkbox" checked={has()} readOnly />
                              {role.name}
                            </div>
                          );
                        }}
                      </For>
                    </div>
                  </Show>
                </div>
              )}
            </For>
          </div>
        </Show>

        {/* Roles Tab */}
        <Show when={activeTab() === "roles"}>
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
                      style={{ background: role.color ? intToHex(role.color) : "var(--color-xfire-text-dim)" }}
                    />
                    <RoleTag name={role.name} color={role.color} />
                    <span class="settings-role-position">pos: {role.position}</span>
                    <Show when={role.hoist}>
                      <span class="settings-role-position">hoisted</span>
                    </Show>
                    <div class="settings-role-actions">
                      <Show when={role.id > 1}>
                        <button
                          class="settings-danger-btn"
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
                    <div class="settings-field">
                      <label class="settings-field-label">Name</label>
                      <input
                        class="settings-input"
                        type="text"
                        value={editRoleName()}
                        onInput={(e) => setEditRoleName(e.currentTarget.value)}
                      />
                    </div>
                    <div class="settings-field">
                      <label class="settings-field-label">Color</label>
                      <div class="settings-field-row">
                        <input
                          type="color"
                          value={editRoleColor()}
                          onInput={(e) => setEditRoleColor(e.currentTarget.value)}
                        />
                        <span class="settings-value">{editRoleColor()}</span>
                      </div>
                    </div>
                    <div class="settings-field">
                      <label class="settings-field-label">Options</label>
                      <div class="settings-field-row">
                        <label class="settings-option">
                          <input
                            type="checkbox"
                            checked={editRoleHoist()}
                            onChange={(e) => setEditRoleHoist(e.currentTarget.checked)}
                          />
                          Hoist (show separately in member list)
                        </label>
                      </div>
                      <div class="settings-field-row">
                        <label class="settings-option">
                          <input
                            type="checkbox"
                            checked={editRoleMentionable()}
                            onChange={(e) => setEditRoleMentionable(e.currentTarget.checked)}
                          />
                          Mentionable
                        </label>
                      </div>
                    </div>
                    <div class="settings-field">
                      <label class="settings-field-label">Permissions</label>
                      <For each={PERMISSION_CATEGORIES}>
                        {(category) => (
                          <div>
                            <div class="settings-hint" style={{ "font-weight": "600", "font-style": "normal" }}>{category.name}</div>
                            <For each={category.permissions}>
                              {(perm) => (
                                <label class="settings-option">
                                  <input
                                    type="checkbox"
                                    checked={hasPerm(editRolePerms(), perm.value)}
                                    onChange={() => setEditRolePerms(togglePermBit(editRolePerms(), perm.value))}
                                  />
                                  {perm.label}
                                </label>
                              )}
                            </For>
                          </div>
                        )}
                      </For>
                    </div>
                    <div class="settings-field-row">
                      <button class="settings-save-btn" onClick={handleSaveEditRole}>
                        <span class="nf-icon">{ICON_SAVE}</span> Save
                      </button>
                      <button class="settings-action-btn" onClick={() => setEditingRoleId(null)}>
                        <span class="nf-icon">{ICON_CLOSE}</span> Cancel
                      </button>
                    </div>
                  </div>
                </Show>
              )}
            </For>
            <Show when={showNewRole()} fallback={
              <button
                class="settings-action-btn"
                onClick={() => setShowNewRole(true)}
              >
                <span class="nf-icon">{ICON_PLUS_BOX}</span> Create Role
              </button>
            }>
              <div class="settings-section" style={{ "padding-left": "8px", "border-left": "2px solid var(--color-xfire-online)" }}>
                <div class="settings-field">
                  <label class="settings-field-label">Name</label>
                  <input
                    class="settings-input"
                    type="text"
                    placeholder="Role name..."
                    value={newRoleName()}
                    onInput={(e) => setNewRoleName(e.currentTarget.value)}
                  />
                </div>
                <div class="settings-field">
                  <label class="settings-field-label">Color</label>
                  <div class="settings-field-row">
                    <input
                      type="color"
                      value={newRoleColor()}
                      onInput={(e) => setNewRoleColor(e.currentTarget.value)}
                    />
                    <span class="settings-value">{newRoleColor()}</span>
                  </div>
                </div>
                <div class="settings-field">
                  <label class="settings-field-label">Options</label>
                  <div class="settings-field-row">
                    <label class="settings-option">
                      <input
                        type="checkbox"
                        checked={newRoleHoist()}
                        onChange={(e) => setNewRoleHoist(e.currentTarget.checked)}
                      />
                      Hoist (show separately)
                    </label>
                  </div>
                  <div class="settings-field-row">
                    <label class="settings-option">
                      <input
                        type="checkbox"
                        checked={newRoleMentionable()}
                        onChange={(e) => setNewRoleMentionable(e.currentTarget.checked)}
                      />
                      Mentionable
                    </label>
                  </div>
                </div>
                <div class="settings-field">
                  <label class="settings-field-label">Permissions</label>
                  <For each={PERMISSION_CATEGORIES}>
                    {(category) => (
                      <div>
                        <div class="settings-hint" style={{ "font-weight": "600", "font-style": "normal" }}>{category.name}</div>
                        <For each={category.permissions}>
                          {(perm) => (
                            <label class="settings-option">
                              <input
                                type="checkbox"
                                checked={hasPerm(newRolePerms(), perm.value)}
                                onChange={() => setNewRolePerms(togglePermBit(newRolePerms(), perm.value))}
                              />
                              {perm.label}
                            </label>
                          )}
                        </For>
                      </div>
                    )}
                  </For>
                </div>
                <div class="settings-field-row">
                  <button
                    class="settings-save-btn"
                    onClick={handleCreateNewRole}
                    disabled={!newRoleName().trim() || creatingRole()}
                  >
                    <span class="nf-icon">{ICON_SAVE}</span> {creatingRole() ? "Creating..." : "Create"}
                  </button>
                  <button
                    class="settings-action-btn"
                    onClick={() => setShowNewRole(false)}
                  >
                    <span class="nf-icon">{ICON_CLOSE}</span> Cancel
                  </button>
                </div>
              </div>
            </Show>
          </div>
        </Show>

        {/* Bans Tab */}
        <Show when={activeTab() === "bans"}>
          <div class="settings-section">
            <Show when={banList().length > 0} fallback={
              <div class="settings-hint">No banned members.</div>
            }>
              <For each={banList()}>
                {(banned) => (
                  <div class="ban-list-item">
                    <div class="ban-list-info">
                      <span class="ban-list-name">{banned.displayName || banned.pseudonymKey.slice(0, 16)}</span>
                      <span class="ban-list-date">
                        Banned {new Date(banned.bannedAt * 1000).toLocaleDateString()}
                      </span>
                    </div>
                    <button
                      class="settings-action-btn"
                      onClick={() => handleUnban(banned.pseudonymKey)}
                    >
                      <span class="nf-icon">{ICON_REFRESH}</span> Unban
                    </button>
                  </div>
                )}
              </For>
            </Show>
          </div>
        </Show>

        {/* Security Tab */}
        <Show when={activeTab() === "security"}>
          <div class="settings-section">
            <div class="settings-field">
              <label class="settings-field-label">MEK Generation</label>
              <div class="settings-value">{props.community.mekGeneration}</div>
            </div>
            <div class="settings-field">
              <label class="settings-field-label">Encryption Key Rotation</label>
              <div class="settings-hint">
                Rotating the encryption key generates a new Media Encryption Key (MEK).
                All members will automatically receive the new key. Messages encrypted
                with previous keys remain readable.
              </div>
              <button class="settings-danger-btn" onClick={confirmRotateKey}>
                <span class="nf-icon">{ICON_KEY}</span> Rotate Encryption Key
              </button>
            </div>
            <div class="settings-field">
              <label class="settings-field-label">Server Status</label>
              <div class="settings-value">
                {props.community.isHosted ? "Hosted by you" : "Remote server"}
              </div>
            </div>
          </div>
        </Show>
      </div>

      <ConfirmDialog
        isOpen={confirmAction() !== null}
        title={confirmAction()?.title ?? ""}
        message={confirmAction()?.message ?? ""}
        danger
        confirmLabel={confirmAction()?.confirmLabel ?? "Confirm"}
        onConfirm={() => { confirmAction()?.action(); setConfirmAction(null); }}
        onCancel={() => setConfirmAction(null)}
      />
    </Modal>
  );
};

export default CommunitySettingsModal;
