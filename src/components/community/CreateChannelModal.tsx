import { Component, createSignal } from "solid-js";
import Modal from "../common/Modal";
import { handleCreateChannel } from "../../handlers/community.handlers";

interface CreateChannelModalProps {
  isOpen: boolean;
  communityId: string;
  onClose: () => void;
}

const CreateChannelModal: Component<CreateChannelModalProps> = (props) => {
  const [name, setName] = createSignal("");
  const [channelType, setChannelType] = createSignal<"text" | "voice">("text");

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const n = name().trim();
    if (!n) return;
    await handleCreateChannel(props.communityId, n, channelType());
    setName("");
    setChannelType("text");
    props.onClose();
  }

  return (
    <Modal isOpen={props.isOpen} title="Create Channel" onClose={props.onClose}>
      <form class="modal-form" onSubmit={handleSubmit}>
        <input
          class="modal-input"
          type="text"
          placeholder="Channel name..."
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
        />
        <select
          class="modal-input"
          value={channelType()}
          onChange={(e) => setChannelType(e.currentTarget.value as "text" | "voice")}
        >
          <option value="text">Text Channel</option>
          <option value="voice">Voice Channel</option>
        </select>
        <button class="modal-btn" type="submit" disabled={!name().trim()}>
          Create
        </button>
      </form>
    </Modal>
  );
};

export default CreateChannelModal;
