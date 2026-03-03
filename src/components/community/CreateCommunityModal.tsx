import { Component, Show, createSignal } from "solid-js";
import { handleCreateCommunity } from "../../handlers/community.handlers";

interface CreateCommunityModalProps {
  isOpen: boolean;
  onClose: () => void;
}

const CreateCommunityModal: Component<CreateCommunityModalProps> = (props) => {
  const [name, setName] = createSignal("");
  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal("");

  const handleSubmit = async () => {
    const trimmed = name().trim();
    if (!trimmed) {
      setError("Community name is required");
      return;
    }
    setError("");
    setSubmitting(true);
    try {
      await handleCreateCommunity(trimmed);
      setName("");
      props.onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmitting(false);
    }
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !submitting()) {
      handleSubmit();
    }
  };

  return (
    <Show when={props.isOpen}>
      <div class="modal-overlay" onClick={() => props.onClose()}>
        <div class="modal-content" onClick={(e) => e.stopPropagation()}>
          <div class="modal-header">
            <span class="modal-title">Create Community</span>
            <button class="modal-close-btn" onClick={() => props.onClose()}>x</button>
          </div>
          <div class="modal-body">
            <input
              class="form-input"
              type="text"
              placeholder="Community name..."
              value={name()}
              onInput={(e) => setName(e.currentTarget.value)}
              onKeyDown={handleKeyDown}
              autofocus
            />

            <Show when={error()}>
              <div class="form-error">{error()}</div>
            </Show>
          </div>
          <div class="modal-footer">
            <button class="form-btn-cancel" onClick={() => props.onClose()}>
              Cancel
            </button>
            <button
              class="form-btn-primary"
              onClick={handleSubmit}
              disabled={submitting() || !name().trim()}
            >
              {submitting() ? "Creating..." : "Create"}
            </button>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default CreateCommunityModal;
