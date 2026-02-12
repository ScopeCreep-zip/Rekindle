import { Component, createSignal } from "solid-js";
import Modal from "../common/Modal";
import { handleCreateCommunity } from "../../handlers/community.handlers";

interface CreateCommunityModalProps {
  isOpen: boolean;
  onClose: () => void;
}

const CreateCommunityModal: Component<CreateCommunityModalProps> = (props) => {
  const [name, setName] = createSignal("");

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const n = name().trim();
    if (!n) return;
    await handleCreateCommunity(n);
    setName("");
    props.onClose();
  }

  return (
    <Modal isOpen={props.isOpen} title="Create Community" onClose={props.onClose}>
      <form class="add-friend-form" onSubmit={handleSubmit}>
        <input
          class="add-friend-input"
          type="text"
          placeholder="Community name..."
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
        />
        <button class="add-friend-btn" type="submit" disabled={!name().trim()}>
          Create
        </button>
      </form>
    </Modal>
  );
};

export default CreateCommunityModal;
