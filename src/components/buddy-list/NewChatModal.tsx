import { Component, Show, createSignal } from "solid-js";
import Modal from "../common/Modal";
import { friendsState, setFriendsState } from "../../stores/friends.store";
import { commands } from "../../ipc/commands";
import { ICON_NEW_CHAT } from "../../icons";

const NewChatModal: Component = () => {
  const [publicKey, setPublicKey] = createSignal("");
  const [displayName, setDisplayName] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);

  function handleClose(): void {
    setFriendsState("showNewChat", false);
    setPublicKey("");
    setDisplayName("");
    setError(null);
  }

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const key = publicKey().trim();
    if (!key) return;
    setError(null);
    const name = displayName().trim() || key.slice(0, 12) + "...";
    try {
      await commands.openChatWindow(key, name);
      handleClose();
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <Modal
      isOpen={friendsState.showNewChat}
      title="New Chat"
      onClose={handleClose}
    >
      <form class="modal-form" onSubmit={handleSubmit}>
        <input
          class="modal-input"
          type="text"
          placeholder="Enter public key..."
          value={publicKey()}
          onInput={(e) => setPublicKey(e.currentTarget.value)}
        />
        <input
          class="modal-input"
          type="text"
          placeholder="Display name (optional)"
          value={displayName()}
          onInput={(e) => setDisplayName(e.currentTarget.value)}
        />
        <Show when={error()}>
          <div class="login-error">{error()}</div>
        </Show>
        <button class="modal-btn" type="submit" disabled={!publicKey().trim()}>
          <span class="nf-icon">{ICON_NEW_CHAT}</span> Start Chat
        </button>
      </form>
    </Modal>
  );
};

export default NewChatModal;
