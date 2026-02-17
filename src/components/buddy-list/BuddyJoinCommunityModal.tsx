import { Component, createSignal } from "solid-js";
import Modal from "../common/Modal";
import { handleJoinCommunity } from "../../handlers/community.handlers";
import { buddyListUI, setBuddyListUI } from "../../stores/buddylist-ui.store";

const BuddyJoinCommunityModal: Component = () => {
  const [communityId, setCommunityId] = createSignal("");
  const [name, setName] = createSignal("");

  function handleClose(): void {
    setBuddyListUI("showJoinCommunity", false);
    setCommunityId("");
    setName("");
  }

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const id = communityId().trim();
    if (!id) return;
    await handleJoinCommunity(id, name().trim() || id.slice(0, 12) + "...");
    handleClose();
  }

  return (
    <Modal isOpen={buddyListUI.showJoinCommunity} title="Join Community" onClose={handleClose}>
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

export default BuddyJoinCommunityModal;
