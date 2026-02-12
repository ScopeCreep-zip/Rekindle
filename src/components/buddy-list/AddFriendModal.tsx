import { Component, Show, createSignal } from "solid-js";
import Modal from "../common/Modal";
import { friendsState, setFriendsState } from "../../stores/friends.store";
import { handleAddFriend } from "../../handlers/buddy.handlers";

const AddFriendModal: Component = () => {
  const [publicKey, setPublicKey] = createSignal("");
  const [message, setMessage] = createSignal("Hey, add me!");
  const [error, setError] = createSignal<string | null>(null);

  function handleClose(): void {
    setFriendsState("showAddFriend", false);
    setPublicKey("");
    setMessage("Hey, add me!");
    setError(null);
  }

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const key = publicKey().trim();
    if (!key) return;
    setError(null);
    const err = await handleAddFriend(key, message().trim());
    if (err) {
      setError(err);
    } else {
      handleClose();
    }
  }

  return (
    <Modal
      isOpen={friendsState.showAddFriend}
      title="Add Friend"
      onClose={handleClose}
    >
      <form class="add-friend-form" onSubmit={handleSubmit}>
        <input
          class="add-friend-input"
          type="text"
          placeholder="Enter public key..."
          value={publicKey()}
          onInput={(e) => setPublicKey(e.currentTarget.value)}
        />
        <input
          class="add-friend-input"
          type="text"
          placeholder="Message (optional)"
          value={message()}
          onInput={(e) => setMessage(e.currentTarget.value)}
        />
        <Show when={error()}>
          <div class="login-error">{error()}</div>
        </Show>
        <button class="add-friend-btn" type="submit" disabled={!publicKey().trim()}>
          Send Friend Request
        </button>
      </form>
    </Modal>
  );
};

export default AddFriendModal;
