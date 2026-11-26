import { Component, For, Show, createEffect, createSignal } from "solid-js";

import Modal from "../common/Modal";
import LoadingButton from "../common/LoadingButton";
import { errorMessage } from "../../utils/error";
import { handleCreateCommunity } from "../../actions/community.actions";
import { handleCreateWithCoCreators, trackCoCreation } from "../../actions/community/co_create";
import { coCreatorState, stopWatchingCreation } from "../../stores/co-creator.store";
import { friendsState } from "../../stores/friends.store";
import CoCreatorStatusView from "./CoCreatorStatusView";
import { CREATE_WAIT_REASON, canCreateCommunity, networkLabel, trackNetworkStatus } from "../../stores/network.store";

// The Host's cap on named co-creators (`MAX_ADDITIONAL_CREATORS`); the
// picker stops there, and the Host still refuses anything past it.
const MAX_CO_CREATORS = 8;

interface CreateCommunityModalProps {
  isOpen: boolean;
  onClose: () => void;
}

const CreateCommunityModal: Component<CreateCommunityModalProps> = (props) => {
  const [name, setName] = createSignal("");
  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal("");
  const [picked, setPicked] = createSignal<string[]>([]);
  // Only accepted friends can be named (plan E3.2 S7s.3, decision (16)).
  const friends = () =>
    Object.values(friendsState.friends).filter((f) => f.friendshipState === "accepted");
  const full = () => picked().length >= MAX_CO_CREATORS;

  function toggle(key: string): void {
    setPicked((p) => (p.includes(key) ? p.filter((k) => k !== key) : [...p, key]));
  }
  // Create waits for the network (plan E7.13, `canCreateCommunity`); the
  // Host still refuses a create that races a drop, and that reason shows.
  trackNetworkStatus();
  // Held invites and pending creations from before this window opened.
  trackCoCreation();

  // Reset on every open so a previous error / name doesn't carry over.
  createEffect(() => {
    if (props.isOpen) {
      setName("");
      setError("");
      setPicked([]);
      setSubmitting(false);
      // A watched creation that finished while the dialog was closed.
      const watching = coCreatorState.watching;
      if (watching && coCreatorState.finished[watching.anchor] !== undefined) stopWatchingCreation();
    }
  });

  async function handleSubmit(e?: Event): Promise<void> {
    e?.preventDefault();
    const trimmed = name().trim();
    if (!trimmed) {
      setError("Community name is required");
      return;
    }
    if (!canCreateCommunity()) return;
    setError("");
    setSubmitting(true);
    try {
      if (picked().length > 0) {
        // The status view takes over until the community is written.
        await handleCreateWithCoCreators(trimmed, picked());
      } else {
        await handleCreateCommunity(trimmed);
        props.onClose();
      }
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Modal isOpen={props.isOpen} title="Create Community" onClose={props.onClose}>
      <Show
        when={coCreatorState.watching}
        fallback={
          <form class="form-group" onSubmit={(e) => void handleSubmit(e)}>
            <input
              class="form-input"
              type="text"
              placeholder="Community name..."
              value={name()}
              onInput={(e) => setName(e.currentTarget.value)}
              autofocus
            />
            <div class="group-dm-picker-hint" id="co-creator-hint">
              Co-creators (optional, friends only, up to {MAX_CO_CREATORS}): {picked().length} chosen
            </div>
            <div class="group-dm-picker" role="group" aria-label="Co-creators" aria-describedby="co-creator-hint">
              <Show
                when={friends().length > 0}
                fallback={<div class="empty-placeholder-subtitle">No friends to name</div>}
              >
                <For each={friends()}>
                  {(friend) => (
                    <label class="group-dm-picker-option">
                      <input
                        type="checkbox"
                        checked={picked().includes(friend.publicKey)}
                        disabled={full() && !picked().includes(friend.publicKey)}
                        onChange={() => toggle(friend.publicKey)}
                      />
                      <span class="buddy-name">{friend.displayName}</span>
                    </label>
                  )}
                </For>
              </Show>
            </div>
            <Show when={!canCreateCommunity()}>
              <div class="form-hint" role="status">
                Waiting for the network ({networkLabel()}): {CREATE_WAIT_REASON}.
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
                disabled={!name().trim() || !canCreateCommunity()}
                loadingLabel="Creating"
              >
                Create
              </LoadingButton>
            </div>
          </form>
        }
      >
        {(watching) => (
          <CoCreatorStatusView
            anchor={watching().anchor}
            name={watching().name}
            onClose={props.onClose}
          />
        )}
      </Show>
    </Modal>
  );
};

export default CreateCommunityModal;
