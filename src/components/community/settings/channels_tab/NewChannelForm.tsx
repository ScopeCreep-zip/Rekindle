import { Component, Show, createSignal } from "solid-js";
import { handleCreateChannel } from "../../../../actions/community.actions";
import { ICON_PLUS_BOX } from "../../../../icons";

type ChannelType = "text" | "voice" | "announcement" | "forum" | "stage" | "directory" | "media" | "events" | "dm";

interface NewChannelFormProps {
  communityId: string;
}

const NewChannelForm: Component<NewChannelFormProps> = (props) => {
  const [showNewChannel, setShowNewChannel] = createSignal(false);
  const [newChannelName, setNewChannelName] = createSignal("");
  const [newChannelType, setNewChannelType] = createSignal<ChannelType>("text");
  const [creatingChannel, setCreatingChannel] = createSignal(false);

  async function handleCreateCh(): Promise<void> {
    const n = newChannelName().trim();
    if (!n) return;
    setCreatingChannel(true);
    try {
      await handleCreateChannel(props.communityId, n, newChannelType());
      setNewChannelName("");
      setNewChannelType("text");
      setShowNewChannel(false);
    } catch {
      // Toast shown by handler; keep form open
    } finally {
      setCreatingChannel(false);
    }
  }

  return (
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
          onChange={(e) => setNewChannelType(e.currentTarget.value as ChannelType)}
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
          class="form-btn-primary"
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
  );
};

export default NewChannelForm;
