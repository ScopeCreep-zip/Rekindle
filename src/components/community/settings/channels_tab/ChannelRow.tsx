import { type Component, For, Show, createSignal, createUniqueId } from "solid-js";
import SkinGlyph from "../../../common/SkinGlyph";
import type { Channel, Community } from "../../../../stores/community.store";
import {
  handleRenameChannel,
  handleDeleteChannel,
  handleSetSlowmode,
  handleSetChannelTopic,
  handleSetChannelForumTags,
  handleReorderChannels,
  handleMoveChannel,
  handleSetNotificationOverride,
  handleSetChannelNotificationSound,
} from "../../../../actions/community.actions";
import ChannelOverwriteEditor from "./ChannelOverwriteEditor";
import type { GlyphName } from "../../../common/glyph_art";

interface ChannelRowProps {
  community: Community;
  channel: Channel;
  canManageChannels: boolean;
}

const ChannelRow: Component<ChannelRowProps> = (props) => {
  const notificationLevelId = createUniqueId();
  const notificationSoundId = createUniqueId();
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

  function deleteChannel(): void {
    void handleDeleteChannel(cid(), props.channel.id);
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

  function channelTypeIcon(): GlyphName {
    return props.channel.type === "voice"
      ? "voice.server"
      : props.channel.type === "announcement"
        ? "megaphone"
        : "chat";
  }

  return (
    <div>
      <div class="channel-manage-row">
        <SkinGlyph name={channelTypeIcon()} class="channel-manage-icon" />
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
              type="button"
              class="form-btn-secondary channel-manage-btn"
              onClick={startRename}
              title="Rename"
            >
              <SkinGlyph name="pencil" />
            </button>
          </Show>
          <Show when={renaming()}>
            <button
              type="button"
              class="form-btn-primary channel-manage-btn"
              onClick={submitRename}
              title="Save"
            >
              <SkinGlyph name="save" />
            </button>
          </Show>
          <button
            type="button"
            class="form-btn-danger channel-manage-btn"
            onClick={deleteChannel}
            title="Delete"
          >
            <SkinGlyph name="delete" />
          </button>
          <button
            type="button"
            class="form-btn-secondary channel-manage-btn"
            onClick={() => setShowOverwrite((v) => !v)}
            title="Permissions"
          >
            <SkinGlyph name="perms" />
          </button>
          <button
            type="button"
            class="form-btn-secondary channel-manage-btn"
            onClick={moveChannelUp}
            disabled={index() === 0}
            title="Move Up"
          >
            <SkinGlyph name="arrow-up" />
          </button>
          <button
            type="button"
            class="form-btn-secondary channel-manage-btn"
            onClick={moveChannelDown}
            disabled={index() === props.community.channels.length - 1}
            title="Move Down"
          >
            <SkinGlyph name="arrow-down" />
          </button>
        </Show>
      </div>
      {/* Category (plan E7.15): the Host's ChannelMove, reviewed by the intent gate. */}
      <Show when={props.canManageChannels}>
        <div class="channel-settings-inline">
          <label class="channel-settings-inline-row">
            <span class="form-field-label">Category</span>
            <select
              class="form-select"
              value={props.channel.categoryId ?? ""}
              onChange={(e) => void handleMoveChannel(
                cid(),
                props.channel.id,
                e.currentTarget.value === "" ? null : e.currentTarget.value,
              )}
            >
              <option value="">No category</option>
              <For each={props.community.categories}>
                {(cat) => <option value={cat.id}>{cat.name}</option>}
              </For>
            </select>
          </label>
        </div>
      </Show>
      {/* Slowmode label / inline editor */}
      <Show when={props.canManageChannels}>
        <div class="channel-settings-inline">
          <Show when={editingSlowmode()} fallback={
            <span class="channel-settings-inline-row">
              <Show when={props.channel.slowmodeSeconds}>
                <span class="channel-slowmode-label">{props.channel.slowmodeSeconds}s slowmode</span>
              </Show>
              <button
                type="button"
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
                onInput={(e) => setSlowmodeValue(parseInt(e.currentTarget.value, 10) || 0)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") submitSlowmode();
                  if (e.key === "Escape") cancelSlowmode();
                }}
                placeholder="Seconds"
              />
              <button type="button" class="form-btn-primary channel-manage-btn" onClick={submitSlowmode}>
                <SkinGlyph name="save" />
              </button>
              <button type="button" class="form-btn-secondary channel-manage-btn" onClick={cancelSlowmode}>
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
                type="button"
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
              <button type="button" class="form-btn-primary channel-manage-btn" onClick={submitTopic}>
                <SkinGlyph name="save" />
              </button>
              <button type="button" class="form-btn-secondary channel-manage-btn" onClick={cancelTopic}>
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
          <label class="channel-notif-label" for={notificationLevelId}>Notifications:</label>
          <select
            class="form-select channel-notif-select"
            id={notificationLevelId}
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
          <label class="channel-notif-label" for={notificationSoundId}>Sound:</label>
          <select
            class="form-select channel-notif-select"
            id={notificationSoundId}
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
                type="button"
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
              <button type="button" class="form-btn-primary channel-manage-btn" onClick={submitForumTags}>
                <SkinGlyph name="save" />
              </button>
              <button type="button" class="form-btn-secondary channel-manage-btn" onClick={cancelForumTags}>
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
