import { Component, createSignal, createEffect, createMemo, For, Show } from "solid-js";
import Modal from "../common/Modal";
import {
  handleCreateChannel,
  handleSetChannelForumTags,
} from "../../handlers/community.handlers";
import { communityState } from "../../stores/community.store";

interface CreateChannelModalProps {
  isOpen: boolean;
  communityId: string;
  onClose: () => void;
}

const CreateChannelModal: Component<CreateChannelModalProps> = (props) => {
  const [name, setName] = createSignal("");
  const [channelType, setChannelType] = createSignal<"text" | "voice" | "announcement" | "forum" | "stage">("text");
  const [parentVoiceChannelId, setParentVoiceChannelId] = createSignal<string>("");
  // Architecture §28.6 — forum channels filter by client-side tag list
  // attached to the channel record. Tags are committed via
  // `setChannelForumTags` immediately after the channel is created so
  // the first member to open the forum view sees them.
  const [forumTags, setForumTags] = createSignal<string[]>([]);
  const [tagInput, setTagInput] = createSignal("");

  createEffect(() => {
    if (props.isOpen) {
      setName("");
      setChannelType("text");
      setParentVoiceChannelId("");
      setForumTags([]);
      setTagInput("");
    }
  });

  // Architecture §10.8 — only text channels may attach to a voice parent.
  const voiceChannels = createMemo(() => {
    const community = communityState.communities[props.communityId];
    if (!community) return [];
    return community.channels.filter((ch) => ch.type === "voice");
  });

  function commitTagInput(): void {
    const next = tagInput().trim();
    if (!next) return;
    if (forumTags().includes(next)) {
      setTagInput("");
      return;
    }
    setForumTags([...forumTags(), next]);
    setTagInput("");
  }

  function handleTagKeyDown(e: KeyboardEvent): void {
    if (e.key === "Enter" || e.key === ",") {
      e.preventDefault();
      commitTagInput();
    } else if (e.key === "Backspace" && tagInput() === "" && forumTags().length > 0) {
      setForumTags(forumTags().slice(0, -1));
    }
  }

  function removeTag(tag: string): void {
    setForumTags(forumTags().filter((t) => t !== tag));
  }

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const n = name().trim();
    if (!n) return;
    try {
      const parent = channelType() === "text" && parentVoiceChannelId()
        ? parentVoiceChannelId()
        : undefined;
      const newChannelId = await handleCreateChannel(
        props.communityId,
        n,
        channelType(),
        undefined,
        parent,
      );
      if (channelType() === "forum" && forumTags().length > 0) {
        await handleSetChannelForumTags(props.communityId, newChannelId, forumTags());
      }
      setName("");
      setChannelType("text");
      setParentVoiceChannelId("");
      setForumTags([]);
      setTagInput("");
      props.onClose();
    } catch {
      // Modal stays open; toast already shown by handler
    }
  }

  return (
    <Modal isOpen={props.isOpen} title="Create Channel" onClose={props.onClose}>
      <form class="form-group" onSubmit={handleSubmit}>
        <input
          class="form-input"
          type="text"
          placeholder="Channel name..."
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
        />
        <select
          class="form-select"
          value={channelType()}
          onChange={(e) => setChannelType(e.currentTarget.value as "text" | "voice" | "announcement" | "forum" | "stage")}
        >
          <option value="text">Text Channel</option>
          <option value="voice">Voice Channel</option>
          <option value="announcement">Announcement</option>
          <option value="forum">Forum</option>
          <option value="stage">Stage</option>
        </select>
        <Show when={channelType() === "text" && voiceChannels().length > 0}>
          <select
            class="form-select"
            value={parentVoiceChannelId()}
            onChange={(e) => setParentVoiceChannelId(e.currentTarget.value)}
            title="Attach this text channel to a voice channel — only visible to active voice members"
          >
            <option value="">No voice parent (regular text channel)</option>
            <For each={voiceChannels()}>
              {(ch) => <option value={ch.id}>Text-in-voice: {ch.name}</option>}
            </For>
          </select>
        </Show>
        <Show when={channelType() === "forum"}>
          <div class="form-field-label">Forum tags</div>
          <div class="forum-tag-chip-input">
            <For each={forumTags()}>
              {(tag) => (
                <span class="forum-tag-chip">
                  {tag}
                  <button
                    class="forum-tag-chip-remove"
                    type="button"
                    onClick={() => removeTag(tag)}
                    aria-label={`Remove tag ${tag}`}
                  >
                    ×
                  </button>
                </span>
              )}
            </For>
            <input
              class="forum-tag-chip-input-field"
              type="text"
              placeholder={forumTags().length === 0 ? "Press Enter to add a tag…" : ""}
              value={tagInput()}
              onInput={(e) => setTagInput(e.currentTarget.value)}
              onKeyDown={handleTagKeyDown}
              onBlur={commitTagInput}
              aria-label="Forum tag input"
            />
          </div>
          <div class="settings-hint">
            Tags filter forum posts inside the channel. Only members with
            <code> MANAGE_CHANNELS </code> can change them later.
          </div>
        </Show>
        <button class="form-btn-primary" type="submit" disabled={!name().trim()}>
          Create
        </button>
      </form>
    </Modal>
  );
};

export default CreateChannelModal;
