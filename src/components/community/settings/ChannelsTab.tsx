import { Component, For, Show, createSignal } from "solid-js";
import type { Community } from "../../../stores/community.store";
import type { ConfirmOptions } from "../CommunitySettingsModal";
import { commands } from "../../../ipc/commands";
import {
  handleRenameChannel,
  handleCreateChannel,
  handleDeleteChannel,
  handleSetSlowmode,
  handleSetChannelTopic,
  handleReorderChannels,
  handleReorderCategories,
  handleCreateCategory,
  handleDeleteCategory,
  handleRenameCategory,
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
  ICON_MEGAPHONE,
  ICON_ARROW_UP,
  ICON_ARROW_DOWN,
  ICON_FOLDER,
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
  const [newChannelType, setNewChannelType] = createSignal<"text" | "voice" | "announcement" | "forum" | "stage" | "directory" | "media" | "events" | "dm">("text");
  const [creatingChannel, setCreatingChannel] = createSignal(false);
  const [overwriteChannelId, setOverwriteChannelId] = createSignal<string | null>(null);
  const [overwriteTargetType, setOverwriteTargetType] = createSignal("role");
  const [overwriteTargetId, setOverwriteTargetId] = createSignal("");
  const [overwriteAllow, setOverwriteAllow] = createSignal(0n);
  const [overwriteDeny, setOverwriteDeny] = createSignal(0n);
  const [editSlowmodeId, setEditSlowmodeId] = createSignal<string | null>(null);
  const [slowmodeValue, setSlowmodeValue] = createSignal(0);
  const [editTopicId, setEditTopicId] = createSignal<string | null>(null);
  const [topicValue, setTopicValue] = createSignal("");

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
    } catch {
      // Toast shown by handler; keep form open
    } finally {
      setCreatingChannel(false);
    }
  }

  function startEditSlowmode(channel: { id: string; slowmodeSeconds?: number }): void {
    setEditSlowmodeId(channel.id);
    setSlowmodeValue(channel.slowmodeSeconds ?? 0);
  }

  async function submitSlowmode(channelId: string): Promise<void> {
    await handleSetSlowmode(props.community.id, channelId, slowmodeValue());
    setEditSlowmodeId(null);
  }

  function cancelSlowmode(): void {
    setEditSlowmodeId(null);
    setSlowmodeValue(0);
  }

  function startEditTopic(channel: { id: string; topic?: string }): void {
    setEditTopicId(channel.id);
    setTopicValue(channel.topic ?? "");
  }

  async function submitTopic(channelId: string): Promise<void> {
    await handleSetChannelTopic(props.community.id, channelId, topicValue());
    setEditTopicId(null);
  }

  function cancelTopic(): void {
    setEditTopicId(null);
    setTopicValue("");
  }

  function moveChannelUp(index: number): void {
    if (index <= 0) return;
    const ids = props.community.channels.map((ch) => ch.id);
    [ids[index - 1], ids[index]] = [ids[index], ids[index - 1]];
    handleReorderChannels(props.community.id, ids);
  }

  function moveChannelDown(index: number): void {
    if (index >= props.community.channels.length - 1) return;
    const ids = props.community.channels.map((ch) => ch.id);
    [ids[index], ids[index + 1]] = [ids[index + 1], ids[index]];
    handleReorderChannels(props.community.id, ids);
  }

  function moveCategoryUp(index: number): void {
    const sorted = [...props.community.categories].sort((a, b) => a.sortOrder - b.sortOrder);
    if (index <= 0) return;
    const ids = sorted.map((c) => c.id);
    [ids[index - 1], ids[index]] = [ids[index], ids[index - 1]];
    handleReorderCategories(props.community.id, ids);
  }

  function moveCategoryDown(index: number): void {
    const sorted = [...props.community.categories].sort((a, b) => a.sortOrder - b.sortOrder);
    if (index >= sorted.length - 1) return;
    const ids = sorted.map((c) => c.id);
    [ids[index], ids[index + 1]] = [ids[index + 1], ids[index]];
    handleReorderCategories(props.community.id, ids);
  }

  const [renamingCategoryId, setRenamingCategoryId] = createSignal<string | null>(null);
  const [categoryRenameValue, setCategoryRenameValue] = createSignal("");
  const [showNewCategory, setShowNewCategory] = createSignal(false);
  const [newCategoryName, setNewCategoryName] = createSignal("");

  function startCategoryRename(cat: { id: string; name: string }): void {
    setRenamingCategoryId(cat.id);
    setCategoryRenameValue(cat.name);
  }

  async function submitCategoryRename(categoryId: string): Promise<void> {
    const val = categoryRenameValue().trim();
    if (val) {
      await handleRenameCategory(props.community.id, categoryId, val);
    }
    setRenamingCategoryId(null);
  }

  function confirmDeleteCategory(cat: { id: string; name: string }): void {
    props.requestConfirm({
      title: "Delete Category",
      message: `Delete category "${cat.name}"? Channels will become uncategorized.`,
      confirmLabel: "Delete",
      action: () => handleDeleteCategory(props.community.id, cat.id),
    });
  }

  async function handleCreateCat(): Promise<void> {
    const n = newCategoryName().trim();
    if (!n) return;
    await handleCreateCategory(props.community.id, n);
    setNewCategoryName("");
    setShowNewCategory(false);
  }

  // Raw bit check for overwrite grid (no admin bypass — overwrites are explicit)
  function hasPerm(perms: bigint, bit: bigint): boolean {
    return (perms & bit) !== 0n;
  }

  function channelNameById(id: string): string {
    return props.community.channels.find((ch) => ch.id === id)?.name ?? id;
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
        Number(overwriteAllow()),
        Number(overwriteDeny()),
      );
      addToast(`Permission overwrite saved for #${channelNameById(channelId)}`, "success");
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
      setOverwriteAllow(0n);
      setOverwriteDeny(0n);
      addToast(`Permission overwrite removed for #${channelNameById(channelId)}`, "success");
    } catch (e) {
      console.error("Failed to delete overwrite:", e);
      addToast("Failed to delete overwrite", "error");
    }
  }

  const sortedCategories = () =>
    [...props.community.categories].sort((a, b) => a.sortOrder - b.sortOrder);

  return (
    <div class="settings-section">
      {/* Category management */}
      <Show when={props.canManageChannels}>
        <div class="settings-subsection">
          <h4 class="settings-subsection-title">Categories</h4>
          <For each={sortedCategories()}>
            {(cat, index) => (
              <div class="channel-manage-row">
                <span class="nf-icon channel-manage-icon">{ICON_FOLDER}</span>
                <Show when={renamingCategoryId() === cat.id} fallback={
                  <span class="channel-manage-name">{cat.name}</span>
                }>
                  <input
                    class="form-input channel-rename-input"
                    type="text"
                    value={categoryRenameValue()}
                    onInput={(e) => setCategoryRenameValue(e.currentTarget.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") submitCategoryRename(cat.id);
                      if (e.key === "Escape") setRenamingCategoryId(null);
                    }}
                  />
                </Show>
                <Show when={renamingCategoryId() !== cat.id}>
                  <button
                    class="form-btn-secondary channel-manage-btn"
                    onClick={() => startCategoryRename(cat)}
                    title="Rename"
                  >
                    <span class="nf-icon">{ICON_PENCIL}</span>
                  </button>
                </Show>
                <Show when={renamingCategoryId() === cat.id}>
                  <button
                    class="form-btn-save channel-manage-btn"
                    onClick={() => submitCategoryRename(cat.id)}
                    title="Save"
                  >
                    <span class="nf-icon">{ICON_SAVE}</span>
                  </button>
                </Show>
                <button
                  class="form-btn-danger channel-manage-btn"
                  onClick={() => confirmDeleteCategory(cat)}
                  title="Delete"
                >
                  <span class="nf-icon">{ICON_DELETE}</span>
                </button>
                <button
                  class="form-btn-secondary channel-manage-btn"
                  onClick={() => moveCategoryUp(index())}
                  disabled={index() === 0}
                  title="Move Up"
                >
                  <span class="nf-icon">{ICON_ARROW_UP}</span>
                </button>
                <button
                  class="form-btn-secondary channel-manage-btn"
                  onClick={() => moveCategoryDown(index())}
                  disabled={index() === sortedCategories().length - 1}
                  title="Move Down"
                >
                  <span class="nf-icon">{ICON_ARROW_DOWN}</span>
                </button>
              </div>
            )}
          </For>
          <Show when={showNewCategory()} fallback={
            <button
              class="form-btn-secondary"
              onClick={() => setShowNewCategory(true)}
            >
              <span class="nf-icon">{ICON_PLUS_BOX}</span> Create Category
            </button>
          }>
            <div class="channel-create-inline">
              <input
                class="form-input"
                type="text"
                placeholder="Category name..."
                value={newCategoryName()}
                onInput={(e) => setNewCategoryName(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleCreateCat();
                  if (e.key === "Escape") setShowNewCategory(false);
                }}
              />
              <button
                class="form-btn-save"
                onClick={handleCreateCat}
                disabled={!newCategoryName().trim()}
              >
                Create
              </button>
              <button
                class="form-btn-secondary"
                onClick={() => setShowNewCategory(false)}
              >
                Cancel
              </button>
            </div>
          </Show>
        </div>
      </Show>

      {/* Channels grouped by category */}
      <h4 class="settings-subsection-title">Channels</h4>
      <For each={(() => {
        const categories = props.community.categories;
        const channels = props.community.channels;
        const groups: { label: string; channels: typeof channels }[] = [];

        // Group channels by category
        const catMap = new Map(categories.map((c) => [c.id, c.name]));
        const grouped = new Map<string | undefined, typeof channels>();
        for (const ch of channels) {
          const key = ch.categoryId;
          const arr = grouped.get(key) ?? [];
          arr.push(ch);
          grouped.set(key, arr);
        }

        // Sorted categories first
        for (const cat of [...categories].sort((a, b) => a.sortOrder - b.sortOrder)) {
          const chs = grouped.get(cat.id);
          if (chs?.length) {
            groups.push({ label: catMap.get(cat.id) ?? cat.id, channels: chs });
            grouped.delete(cat.id);
          }
        }

        // Uncategorized last
        const uncategorized = [...(grouped.get(undefined) ?? []), ...[...grouped.entries()].filter(([k]) => k !== undefined).flatMap(([, v]) => v)];
        if (uncategorized.length) {
          groups.push({ label: categories.length > 0 ? "(Uncategorized)" : "All Channels", channels: uncategorized });
        }

        return groups;
      })()}>
        {(group) => (
          <>
            <div class="channel-category-label">{group.label}</div>
            <For each={group.channels}>
              {(channel) => {
                const index = () => props.community.channels.indexOf(channel);
                return (
          <div>
            <div class="channel-manage-row">
              <span class="nf-icon channel-manage-icon">
                {channel.type === "voice" ? ICON_VOLUME_HIGH : channel.type === "announcement" ? ICON_MEGAPHONE : ICON_CHANNEL_TEXT}
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
                    setOverwriteAllow(0n);
                    setOverwriteDeny(0n);
                  }}
                  title="Permissions"
                >
                  <span class="nf-icon">{ICON_PERMS}</span>
                </button>
                <button
                  class="form-btn-secondary channel-manage-btn"
                  onClick={() => moveChannelUp(index())}
                  disabled={index() === 0}
                  title="Move Up"
                >
                  <span class="nf-icon">{ICON_ARROW_UP}</span>
                </button>
                <button
                  class="form-btn-secondary channel-manage-btn"
                  onClick={() => moveChannelDown(index())}
                  disabled={index() === props.community.channels.length - 1}
                  title="Move Down"
                >
                  <span class="nf-icon">{ICON_ARROW_DOWN}</span>
                </button>
              </Show>
            </div>
            {/* Slowmode label / inline editor */}
            <Show when={props.canManageChannels}>
              <div class="channel-settings-inline">
                <Show when={editSlowmodeId() === channel.id} fallback={
                  <span class="channel-settings-inline-row">
                    <Show when={channel.slowmodeSeconds}>
                      <span class="channel-slowmode-label">{channel.slowmodeSeconds}s slowmode</span>
                    </Show>
                    <button
                      class="form-btn-secondary channel-manage-btn"
                      onClick={() => startEditSlowmode(channel)}
                      title="Set Slowmode"
                    >
                      Slowmode
                    </button>
                  </span>
                }>
                  <span class="channel-settings-inline-row">
                    <input
                      class="form-input channel-slowmode-input"
                      type="number"
                      min="0"
                      value={slowmodeValue()}
                      onInput={(e) => setSlowmodeValue(parseInt(e.currentTarget.value) || 0)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") submitSlowmode(channel.id);
                        if (e.key === "Escape") cancelSlowmode();
                      }}
                      placeholder="Seconds"
                    />
                    <button class="form-btn-save channel-manage-btn" onClick={() => submitSlowmode(channel.id)}>
                      <span class="nf-icon">{ICON_SAVE}</span>
                    </button>
                    <button class="form-btn-secondary channel-manage-btn" onClick={cancelSlowmode}>
                      Cancel
                    </button>
                  </span>
                </Show>
              </div>
            </Show>
            {/* Topic inline editor */}
            <Show when={props.canManageChannels}>
              <div class="channel-settings-inline">
                <Show when={editTopicId() === channel.id} fallback={
                  <span class="channel-settings-inline-row">
                    <Show when={channel.topic}>
                      <span class="channel-topic-label">{channel.topic}</span>
                    </Show>
                    <button
                      class="form-btn-secondary channel-manage-btn"
                      onClick={() => startEditTopic(channel)}
                      title="Set Topic"
                    >
                      Topic
                    </button>
                  </span>
                }>
                  <span class="channel-settings-inline-row">
                    <input
                      class="form-input channel-topic-input"
                      type="text"
                      value={topicValue()}
                      onInput={(e) => setTopicValue(e.currentTarget.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") submitTopic(channel.id);
                        if (e.key === "Escape") cancelTopic();
                      }}
                      placeholder="Channel topic..."
                    />
                    <button class="form-btn-save channel-manage-btn" onClick={() => submitTopic(channel.id)}>
                      <span class="nf-icon">{ICON_SAVE}</span>
                    </button>
                    <button class="form-btn-secondary channel-manage-btn" onClick={cancelTopic}>
                      Cancel
                    </button>
                  </span>
                </Show>
              </div>
            </Show>
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
                );
              }}
            </For>
          </>
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
              onChange={(e) => setNewChannelType(e.currentTarget.value as "text" | "voice" | "announcement")}
            >
              <option value="text">Text</option>
              <option value="voice">Voice</option>
              <option value="announcement">Announcement</option>
              <option value="forum">Forum</option>
              <option value="stage">Stage</option>
              <option value="directory">Directory</option>
              <option value="media">Media</option>
              <option value="events">Events</option>
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
