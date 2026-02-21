import { Component, createSignal, createEffect } from "solid-js";
import Modal from "../common/Modal";
import { handleRenameChannel } from "../../handlers/community.handlers";

interface RenameChannelModalProps {
  isOpen: boolean;
  communityId: string;
  channelId: string;
  currentName: string;
  onClose: () => void;
}

const RenameChannelModal: Component<RenameChannelModalProps> = (props) => {
  const [name, setName] = createSignal("");

  createEffect(() => {
    if (props.isOpen) {
      setName(props.currentName);
    }
  });

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const n = name().trim();
    if (!n || n === props.currentName) return;
    await handleRenameChannel(props.communityId, props.channelId, n);
    props.onClose();
  }

  return (
    <Modal isOpen={props.isOpen} title="Rename Channel" onClose={props.onClose}>
      <form class="modal-form" onSubmit={handleSubmit}>
        <input
          class="modal-input"
          type="text"
          placeholder="Channel name..."
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
        />
        <button
          class="modal-btn"
          type="submit"
          disabled={!name().trim() || name().trim() === props.currentName}
        >
          Rename
        </button>
      </form>
    </Modal>
  );
};

export default RenameChannelModal;
