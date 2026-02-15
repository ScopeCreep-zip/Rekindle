import { Component, createSignal } from "solid-js";
import Modal from "../common/Modal";
import { handleCreateCommunity } from "../../handlers/community.handlers";
import { buddyListUI, setBuddyListUI } from "../../stores/buddylist-ui.store";

const BuddyCreateCommunityModal: Component = () => {
  const [name, setName] = createSignal("");

  function handleClose(): void {
    setBuddyListUI("showCreateCommunity", false);
    setName("");
  }

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const n = name().trim();
    if (!n) return;
    await handleCreateCommunity(n);
    handleClose();
  }

  return (
    <Modal isOpen={buddyListUI.showCreateCommunity} title="Create Community" onClose={handleClose}>
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

export default BuddyCreateCommunityModal;
