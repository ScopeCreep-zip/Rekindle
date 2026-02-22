import { Component, Show, createSignal } from "solid-js";
import { handleAddFriend } from "../../handlers/buddy.handlers";

interface PublicKeyTabProps {
  onClose: () => void;
}

const PublicKeyTab: Component<PublicKeyTabProps> = (props) => {
  const [publicKey, setPublicKey] = createSignal("");
  const [message, setMessage] = createSignal("Hey, add me!");
  const [error, setError] = createSignal<string | null>(null);

  async function handleKeySubmit(e: Event): Promise<void> {
    e.preventDefault();
    const key = publicKey().trim();
    if (!key) return;
    setError(null);
    const err = await handleAddFriend(key, message().trim());
    if (err) {
      setError(err);
    } else {
      props.onClose();
    }
  }

  return (
    <form class="modal-form" onSubmit={handleKeySubmit}>
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
        placeholder="Message (optional)"
        value={message()}
        onInput={(e) => setMessage(e.currentTarget.value)}
      />
      <Show when={error()}>
        <div class="modal-error">{error()}</div>
      </Show>
      <button class="modal-btn" type="submit" disabled={!publicKey().trim()}>
        Send Friend Request
      </button>
    </form>
  );
};

export default PublicKeyTab;
