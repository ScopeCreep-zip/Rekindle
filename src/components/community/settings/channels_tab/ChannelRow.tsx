import { Component, For, Show, createSignal } from "solid-js";
import type { Channel, Community } from "../../../../stores/community.store";
import type { ConfirmOptions } from "../types";
import {
  handleRenameChannel,
  handleDeleteChannel,
  handleSetSlowmode,
  handleSetChannelTopic,
  handleSetChannelForumTags,
  handleReorderChannels,
  handleSetNotificationOverride,
  handleSetChannelNotificationSound,
} from "../../../../actions/community.actions";
import ChannelOverwriteEditor from "./ChannelOverwriteEditor";
import {
  ICON_SAVE,
  ICON_PENCIL,
  ICON_DELETE,
  ICON_PERMS,
  ICON_CHANNEL_TEXT,
  ICON_VOLUME_HIGH,
  ICON_MEGAPHONE,
  ICON_ARROW_UP,
  ICON_ARROW_DOWN,
} from "../../../../icons";

interface ChannelRowProps {
  community: Community;
  channel: Channel;
  canManageChannels: boolean;
  requestConfirm: (opts: ConfirmOptions) => void;
}

const ChannelRow: Component<ChannelRowProps> = (props) => {
  const [renaming, setRenaming] = createSignal(false);
  const [renameValue, setRenameValue] = createSignal("");
  const [editingSlowmode, setEditingSlowmode] = createSignal(false);
  const [slowmodeValue, setSlowmodeValue] = createSignal(0);
  const [editingTopic, setEditingTopic] = createSignal(false);
  const [topicValue, setTopicValue] = createSignal("");
  const [editingForumTags, setEditingForumTags] = createSignal(false);
  const [forumTagsValue, setForumTagsValue] = createSignal("");
  const [showOverwrite, setShowOverwrite] = createSignal(false);

  const cid = () => props.community.id;
  const index = () => props.community.channels.indexOf(props.channel);

  function startRename(): void {
    setRenameValue(props.channel.name);
    setRenaming(true);
  }

  async function submitRename(): Promise<void> {
    const val = renameValue().trim();
    if (val) {
      await handleRenameChannel(cid(), props.channel.id, val);
    }
    setRenaming(false);
  }

  function confirmDeleteChannel(): void {
    props.requestConfirm({
      title: "Delete Channel",
      message: `Delete #${props.channel.name}? This cannot be undone.`,
      confirmLabel: "Delete",
      action: () => handleDeleteChannel(cid(), props.channel.id),
    });
  }

  function startEditSlowmode(): void {
    setSlowmodeValue(props.channel.slowmodeSeconds ?? 0);
    setEditingSlowmode(true);
  }

  async function submitSlowmode(): Promise<void> {
    await handleSetSlowmode(cid(), props.channel.id, slowmodeValue());
    setEditingSlowmode(false);
  }

  function cancelSlowmode(): void {
    setEditingSlowmode(false);
    setSlowmodeValue(0);
  }

  function startEditTopic(): void {
    setTopicValue(props.channel.topic ?? "");
    setEditingTopic(true);
  }

  async function submitTopic(): Promise<void> {
    await handleSetChannelTopic(cid(), props.channel.id, topicValue());
    setEditingTopic(false);
  }

  function cancelTopic(): void {
    setEditingTopic(false);
    setTopicValue("");
  }

  function startEditForumTags(): void {
    setForumTagsValue((props.channel.forumTags ?? []).join(", "));
    setEditingForumTags(true);
  }

  async function submitForumTags(): Promise<void> {
    const tags = forumTagsValue()
      .split(",")
      .map((tag) => tag.trim())
      .filter(Boolean);
    await handleSetChannelForumTags(cid(), props.channel.id, tags);
    setEditingForumTags(false);
  }

  function cancelForumTags(): void {
    setEditingForumTags(false);
    setForumTagsValue("");
  }

  // Architecture §17.1 tier 1 — per-channel notification override is
  // local-only; it does not propagate to peers.
  function changeChannelNotifLevel(level: "all" | "mentions" | "nothing"): void {
    void handleSetNotificationOverride(cid(), props.channel.id, level);
  }

  // Architecture §32 Phase 7 Week 25 — sound override cascade. Stores the
  // soundboard expression's content_hash; empty clears the override.
  function changeChannelSound(soundRef: string): void {
    void handleSetChannelNotificationSound(
      cid(),
      props.channel.id,
      soundRef.length > 0 ? soundRef : null,
    );
  }

  // Soundboard expressions in this community, used to populate the
  // notification-sound picker. Falls back to "(default)" when empty.
  const soundboardSounds = () =>
    (props.community.expressions ?? []).filter((expr) => expr.kind === "soundboard");

  function moveChannelUp(): void {
    if (index() <= 0) return;
    const ids = props.community.channels.map((ch) => ch.id);
    const i = index();
    [ids[i - 1], ids[i]] = [ids[i], ids[i - 1]];
    handleReorderChannels(cid(), ids);
  }

  function moveChannelDown(): void {
    if (index() >= props.community.channels.length - 1) return;
    const ids = props.community.channels.map((ch) => ch.id);
    const i = index();
    [ids[i], ids[i + 1]] = [ids[i + 1], ids[i]];
    handleReorderChannels(cid(), ids);
  }

  function channelTypeIcon() {
    return props.channel.type === "voice"
      ? ICON_VOLUME_HIGH
      : props.channel.type === "announcement"
        ? ICON_MEGAPHONE
        : ICON_CHANNEL_TEXT;
  }

  return (
    <div>
      <div class="channel-manage-row">
        <span class="nf-icon channel-manage-icon">{channelTypeIcon()}</span>
        <Show when={renaming()} fallback={
          <span class="channel-manage-name">{props.channel.name}</span>
        }>
          <input
            class="form-input channel-rename-input"
            type="text"
            value={renameValue()}
            onInput={(e) => setRenameValue(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submitRename();
              if (e.key === "Escape") setRenaming(false);
            }}
          />
        </Show>
        <span class="channel-manage-type">{props.channel.type}</span>
        <Show when={props.canManageChannels}>
          <Show when={!renaming()}>
            <button
              class="form-btn-secondary channel-manage-btn"
              onClick={startRename}
              title="Rename"
            >
              <span class="nf-icon">{ICON_PENCIL}</span>
            </button>
          </Show>
          <Show when={renaming()}>
            <button
              class="form-btn-primary channel-manage-btn"
              onClick={submitRename}
              title="Save"
            >
              <span class="nf-icon">{ICON_SAVE}</span>
            </button>
          </Show>
          <button
            class="form-btn-danger channel-manage-btn"
            onClick={confirmDeleteChannel}
            title="Delete"
          >
            <span class="nf-icon">{ICON_DELETE}</span>
          </button>
          <button
            class="form-btn-secondary channel-manage-btn"
            onClick={() => setShowOverwrite((v) => !v)}
            title="Permissions"
          >
            <span class="nf-icon">{ICON_PERMS}</span>
          </button>
          <button
            class="form-btn-secondary channel-manage-btn"
            onClick={moveChannelUp}
            disabled={index() === 0}
            title="Move Up"
          >
            <span class="nf-icon">{ICON_ARROW_UP}</span>
          </button>
          <button
            class="form-btn-secondary channel-manage-btn"
            onClick={moveChannelDown}
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
          <Show when={editingSlowmode()} fallback={
            <span class="channel-settings-inline-row">
              <Show when={props.channel.slowmodeSeconds}>
                <span class="channel-slowmode-label">{props.channel.slowmodeSeconds}s slowmode</span>
              </Show>
              <button
                class="form-btn-secondary channel-manage-btn"
                onClick={startEditSlowmode}
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
                  if (e.key === "Enter") submitSlowmode();
                  if (e.key === "Escape") cancelSlowmode();
                }}
                placeholder="Seconds"
              />
              <button class="form-btn-primary channel-manage-btn" onClick={submitSlowmode}>
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
          <Show when={editingTopic()} fallback={
            <span class="channel-settings-inline-row">
              <Show when={props.channel.topic}>
                <span class="channel-topic-label">{props.channel.topic}</span>
              </Show>
              <button
                class="form-btn-secondary channel-manage-btn"
                onClick={startEditTopic}
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
                  if (e.key === "Enter") submitTopic();
                  if (e.key === "Escape") cancelTopic();
                }}
                placeholder="Channel topic..."
              />
              <button class="form-btn-primary channel-manage-btn" onClick={submitTopic}>
                <span class="nf-icon">{ICON_SAVE}</span>
              </button>
              <button class="form-btn-secondary channel-manage-btn" onClick={cancelTopic}>
                Cancel
              </button>
            </span>
          </Show>
        </div>
      </Show>
      {/* Architecture §17.1 tier 1 — local per-channel notification
          level + per-channel sound override. Visible to every member,
          not just admins (it's a personal preference). */}
      <div class="channel-settings-inline">
        <span class="channel-settings-inline-row">
          <label class="channel-notif-label">Notifications:</label>
          <select
            class="form-select channel-notif-select"
            value={props.channel.notificationLevel ?? "all"}
            onChange={(e) => {
              const value = e.currentTarget.value as "all" | "mentions" | "nothing";
              void changeChannelNotifLevel(value);
            }}
          >
            <option value="all">All messages</option>
            <option value="mentions">Mentions only</option>
            <option value="nothing">Nothing</option>
          </select>
          <label class="channel-notif-label">Sound:</label>
          <select
            class="form-select channel-notif-select"
            value={props.channel.notificationSoundRef ?? ""}
            onChange={(e) => changeChannelSound(e.currentTarget.value)}
          >
            <option value="">(default)</option>
            <For each={soundboardSounds()}>
              {(sound) => (
                <option value={sound.contentHash}>{sound.name}</option>
              )}
            </For>
          </select>
        </span>
      </div>
      <Show when={props.canManageChannels && props.channel.type === "forum"}>
        <div class="channel-settings-inline">
          <Show when={editingForumTags()} fallback={
            <span class="channel-settings-inline-row">
              <Show when={(props.channel.forumTags?.length ?? 0) > 0}>
                <span class="channel-topic-label">{props.channel.forumTags?.join(", ")}</span>
              </Show>
              <button
                class="form-btn-secondary channel-manage-btn"
                onClick={startEditForumTags}
                title="Set Forum Tags"
              >
                Forum Tags
              </button>
            </span>
          }>
            <span class="channel-settings-inline-row">
              <input
                class="form-input channel-topic-input"
                type="text"
                value={forumTagsValue()}
                onInput={(e) => setForumTagsValue(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") submitForumTags();
                  if (e.key === "Escape") cancelForumTags();
                }}
                placeholder="news, updates, support"
              />
              <button class="form-btn-primary channel-manage-btn" onClick={submitForumTags}>
                <span class="nf-icon">{ICON_SAVE}</span>
              </button>
              <button class="form-btn-secondary channel-manage-btn" onClick={cancelForumTags}>
                Cancel
              </button>
            </span>
          </Show>
        </div>
      </Show>
      <Show when={showOverwrite() && props.canManageChannels}>
        <ChannelOverwriteEditor
          communityId={cid()}
          channelId={props.channel.id}
          channelName={props.channel.name}
          roles={props.community.roles}
        />
      </Show>
    </div>
  );
};

export default ChannelRow;
