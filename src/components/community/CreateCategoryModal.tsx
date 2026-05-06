import { Component, createEffect, createSignal } from "solid-js";

import Modal from "../common/Modal";
import LoadingButton from "../common/LoadingButton";
import { handleCreateCategory } from "../../handlers/community.handlers";

interface CreateCategoryModalProps {
  isOpen: boolean;
  communityId: string;
  onClose: () => void;
}

const CreateCategoryModal: Component<CreateCategoryModalProps> = (props) => {
  const [name, setName] = createSignal("");
  const [loading, setLoading] = createSignal(false);

  createEffect(() => {
    if (props.isOpen) {
      setName("");
      setLoading(false);
    }
  });

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    if (!name().trim()) return;
    setLoading(true);
    try {
      await handleCreateCategory(props.communityId, name().trim());
      props.onClose();
    } finally {
      setLoading(false);
    }
  }

  return (
    <Modal isOpen={props.isOpen} title="Create Category" onClose={props.onClose} size="sm">
      <form onSubmit={(e) => void handleSubmit(e)} class="form-group">
        <input
          class="form-input"
          placeholder="Category name"
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
          autofocus
        />
        <LoadingButton
          type="submit"
          loading={loading()}
          disabled={!name().trim()}
          loadingLabel="Creating"
        >
          Create
        </LoadingButton>
      </form>
    </Modal>
  );
};

export default CreateCategoryModal;
