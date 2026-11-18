import { Component, Show, createEffect, createSignal } from "solid-js";

import Modal from "../common/Modal";
import LoadingButton from "../common/LoadingButton";
import { errorMessage } from "../../utils/error";
import { handleCreateCommunity } from "../../actions/community.actions";
import { isNetworkConnected, networkLabel, trackNetworkStatus } from "../../stores/network.store";

interface CreateCommunityModalProps {
  isOpen: boolean;
  onClose: () => void;
}

const CreateCommunityModal: Component<CreateCommunityModalProps> = (props) => {
  const [name, setName] = createSignal("");
  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal("");
  // Create waits for the network (plan E7.13); the Host still refuses a
  // create that races a drop, and that reason shows below.
  trackNetworkStatus();

  // Reset on every open so a previous error / name doesn't carry over.
  createEffect(() => {
    if (props.isOpen) {
      setName("");
      setError("");
      setSubmitting(false);
    }
  });

  async function handleSubmit(e?: Event): Promise<void> {
    e?.preventDefault();
    const trimmed = name().trim();
    if (!trimmed) {
      setError("Community name is required");
      return;
    }
    if (!isNetworkConnected()) return;
    setError("");
    setSubmitting(true);
    try {
      await handleCreateCommunity(trimmed);
      props.onClose();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Modal isOpen={props.isOpen} title="Create Community" onClose={props.onClose}>
      <form class="form-group" onSubmit={(e) => void handleSubmit(e)}>
        <input
          class="form-input"
          type="text"
          placeholder="Community name..."
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
          autofocus
        />
        <Show when={!isNetworkConnected()}>
          <div class="form-hint" role="status">
            Waiting for the network ({networkLabel()}): a community can be created once connected.
          </div>
        </Show>
        <Show when={error()}>
          <div class="form-error" role="alert">{error()}</div>
        </Show>
        <div class="form-field-row">
          <button
            type="button"
            class="form-btn-secondary"
            onClick={() => props.onClose()}
            disabled={submitting()}
          >
            Cancel
          </button>
          <LoadingButton
            type="submit"
            loading={submitting()}
            disabled={!name().trim() || !isNetworkConnected()}
            loadingLabel="Creating"
          >
            Create
          </LoadingButton>
        </div>
      </form>
    </Modal>
  );
};

export default CreateCommunityModal;
