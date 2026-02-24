import { Component, Show, createSignal } from "solid-js";
import { ICON_CLOSE } from "../../icons";
import { handleCreateCategory } from "../../handlers/community.handlers";

interface CreateCategoryModalProps {
  isOpen: boolean;
  communityId: string;
  onClose: () => void;
}

const CreateCategoryModal: Component<CreateCategoryModalProps> = (props) => {
  const [name, setName] = createSignal("");
  const [loading, setLoading] = createSignal(false);

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    if (!name().trim()) return;
    setLoading(true);
    await handleCreateCategory(props.communityId, name().trim());
    setLoading(false);
    setName("");
    props.onClose();
  }

  return (
    <Show when={props.isOpen}>
      <div class="modal-overlay" onClick={props.onClose}>
        <div class="modal-container modal-container-sm" onClick={(e) => e.stopPropagation()}>
          <div class="modal-header">
            <span class="modal-title">Create Category</span>
            <button class="modal-close-btn" onClick={props.onClose}>
              <span class="nf-icon">{ICON_CLOSE}</span>
            </button>
          </div>
          <div class="modal-body">
            <form onSubmit={handleSubmit} class="form-group">
              <input
                class="form-input"
                placeholder="Category name"
                value={name()}
                onInput={(e) => setName(e.currentTarget.value)}
                autofocus
              />
              <button class="form-btn-primary" type="submit" disabled={loading() || !name().trim()}>
                {loading() ? "Creating..." : "Create"}
              </button>
            </form>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default CreateCategoryModal;
