import { Component, createSignal } from "solid-js";
import Modal from "../common/Modal";
import { handleJoinCommunity } from "../../handlers/community.handlers";

interface JoinCommunityModalProps {
  isOpen: boolean;
  onClose: () => void;
}

const JoinCommunityModal: Component<JoinCommunityModalProps> = (props) => {
  const [communityId, setCommunityId] = createSignal("");
  const [name, setName] = createSignal("");

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const id = communityId().trim();
    if (!id) return;
    await handleJoinCommunity(id, name().trim() || id.slice(0, 12) + "...");
    setCommunityId("");
    setName("");
    props.onClose();
  }

  return (
    <Modal isOpen={props.isOpen} title="Join Community" onClose={props.onClose}>
      <form class="add-friend-form" onSubmit={handleSubmit}>
        <input
          class="add-friend-input"
          type="text"
          placeholder="Community ID..."
          value={communityId()}
          onInput={(e) => setCommunityId(e.currentTarget.value)}
        />
        <input
          class="add-friend-input"
          type="text"
          placeholder="Name (optional)"
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
        />
        <button class="add-friend-btn" type="submit" disabled={!communityId().trim()}>
          Join
        </button>
      </form>
    </Modal>
  );
};

export default JoinCommunityModal;
