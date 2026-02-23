import { Component, For, Show, createSignal } from "solid-js";
import type { Community } from "../../../stores/community.store";
import type { ConfirmOptions } from "../CommunitySettingsModal";
import { commands } from "../../../ipc/commands";
import {
  handleRenameChannel,
  handleCreateChannel,
  handleDeleteChannel,
} from "../../../handlers/community.handlers";
import {
  hasPermission,
  togglePermBit,
  PERMISSION_CATEGORIES,
} from "../../../ipc/permissions";
import { addToast } from "../../../stores/toast.store";
import {
  ICON_SAVE,
  ICON_PENCIL,
  ICON_DELETE,
  ICON_PERMS,
  ICON_PLUS_BOX,
  ICON_CHANNEL_TEXT,
  ICON_VOLUME_HIGH,
} from "../../../icons";

interface ChannelsTabProps {
  community: Community;
  canManageChannels: boolean;
  requestConfirm: (opts: ConfirmOptions) => void;
}

const ChannelsTab: Component<ChannelsTabProps> = (props) => {
  const [renamingChannelId, setRenamingChannelId] = createSignal<string | null>(null);
  const [renameValue, setRenameValue] = createSignal("");
  const [showNewChannel, setShowNewChannel] = createSignal(false);
  const [newChannelName, setNewChannelName] = createSignal("");
  const [newChannelType, setNewChannelType] = createSignal<"text" | "voice">("text");
  const [creatingChannel, setCreatingChannel] = createSignal(false);
  const [overwriteChannelId, setOverwriteChannelId] = createSignal<string | null>(null);
  const [overwriteTargetType, setOverwriteTargetType] = createSignal("role");
  const [overwriteTargetId, setOverwriteTargetId] = createSignal("");
  const [overwriteAllow, setOverwriteAllow] = createSignal(0);
  const [overwriteDeny, setOverwriteDeny] = createSignal(0);

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
    props.requestConfirm({
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

  // Raw bit check for overwrite grid (no admin bypass — overwrites are explicit)
  function hasPerm(perms: number, bit: number): boolean {
    if (bit > 0x7FFF_FFFF) {
      return Math.floor(perms / bit) % 2 === 1;
    }
    return (perms & bit) !== 0;
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
                  class="form-input channel-rename-input"
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
              <Show when={props.canManageChannels}>
                <Show when={renamingChannelId() !== channel.id}>
                  <button
                    class="form-btn-secondary channel-manage-btn"
                    onClick={() => startRename(channel)}
                    title="Rename"
                  >
                    <span class="nf-icon">{ICON_PENCIL}</span>
                  </button>
                </Show>
                <Show when={renamingChannelId() === channel.id}>
                  <button
                    class="form-btn-save channel-manage-btn"
                    onClick={() => submitRename(channel.id)}
                    title="Save"
                  >
                    <span class="nf-icon">{ICON_SAVE}</span>
                  </button>
                </Show>
                <button
                  class="form-btn-danger channel-manage-btn"
                  onClick={() => confirmDeleteChannel(channel)}
                  title="Delete"
                >
                  <span class="nf-icon">{ICON_DELETE}</span>
                </button>
                <button
                  class="form-btn-secondary channel-manage-btn"
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
            <Show when={overwriteChannelId() === channel.id && props.canManageChannels}>
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
                  <div class="form-field-row">
                    <button class="form-btn-save" onClick={handleSaveOverwrite}>
                      <span class="nf-icon">{ICON_SAVE}</span> Save Overwrite
                    </button>
                    <button class="form-btn-danger" onClick={handleDeleteOverwrite}>
                      <span class="nf-icon">{ICON_DELETE}</span> Remove Overwrite
                    </button>
                  </div>
                </Show>
              </div>
            </Show>
          </div>
        )}
      </For>
      <Show when={props.canManageChannels}>
        <Show when={showNewChannel()} fallback={
          <button
            class="form-btn-secondary"
            onClick={() => setShowNewChannel(true)}
          >
            <span class="nf-icon">{ICON_PLUS_BOX}</span> Create Channel
          </button>
        }>
          <div class="channel-create-inline">
            <input
              class="form-input"
              type="text"
              placeholder="Channel name..."
              value={newChannelName()}
              onInput={(e) => setNewChannelName(e.currentTarget.value)}
            />
            <select
              class="form-select channel-type-select"
              value={newChannelType()}
              onChange={(e) => setNewChannelType(e.currentTarget.value as "text" | "voice")}
            >
              <option value="text">Text</option>
              <option value="voice">Voice</option>
            </select>
            <button
              class="form-btn-save"
              onClick={handleCreateCh}
              disabled={!newChannelName().trim() || creatingChannel()}
            >
              {creatingChannel() ? "Creating..." : "Create"}
            </button>
            <button
              class="form-btn-secondary"
              onClick={() => setShowNewChannel(false)}
            >
              Cancel
            </button>
          </div>
        </Show>
      </Show>
    </div>
  );
};

export default ChannelsTab;
