import { Component, For, Show, createEffect, createSignal } from "solid-js";

import { coCreatorState, stopWatchingCreation } from "../../stores/co-creator.store";
import { communityState } from "../../stores/community.store";
import { friendsState } from "../../stores/friends.store";
import { handleCancelCreation, handleRemoveCoCreator } from "../../actions/community/co_create";

interface CoCreatorStatusViewProps {
  anchor: string;
  name: string;
  onClose: () => void;
}

/** The Host's word for a co-creator's state, as a label. */
function statusLabel(status: string): string {
  return status.charAt(0).toUpperCase() + status.slice(1);
}

/** A co-creator the round still waits on can be removed from it. */
function removable(status: string): boolean {
  return status === "pending" || status === "accepted";
}

/**
 * A creation with co-creators (plan E3.2 S7s.3f): each co-creator's state
 * as the Host states it (`coCreatorStatus`), with Remove per co-creator and
 * Cancel for the creation, both through the Host's intent review (ADR
 * 0019). The view closes once the community exists: the Host's
 * `CommunityCreated` re-reads it into the community store.
 */
const CoCreatorStatusView: Component<CoCreatorStatusViewProps> = (props) => {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const rows = () => Object.entries(coCreatorState.statuses[props.anchor] ?? {});
  const nameOf = (peer: string): string =>
    friendsState.friends[peer]?.displayName ?? `${peer.slice(0, 12)}…`;

  createEffect(() => {
    if (communityState.communities[props.anchor]) {
      stopWatchingCreation();
      props.onClose();
    }
  });

  async function run(action: () => Promise<string | null>, closeAfter: boolean): Promise<void> {
    setBusy(true);
    setError(null);
    const err = await action();
    setBusy(false);
    if (err) setError(err);
    else if (closeAfter) {
      stopWatchingCreation();
      props.onClose();
    }
  }

  return (
    <div class="form-group">
      <div class="co-create-status-hint" id="co-create-status-hint">
        Creating {props.name}: it is written once every co-creator has answered or been removed.
      </div>
      <ul class="co-create-status-list" aria-describedby="co-create-status-hint">
        <For each={rows()}>
          {([peer, status]) => (
            <li class="co-create-status-row">
              <span class="buddy-name">{nameOf(peer)}</span>
              <span class={`co-create-status-badge co-create-status-${status}`}>{statusLabel(status)}</span>
              <Show when={removable(status)}>
                <button
                  type="button"
                  class="pending-btn-reject"
                  disabled={busy()}
                  aria-label={`Remove ${nameOf(peer)} as co-creator`}
                  onClick={() => void run(() => handleRemoveCoCreator(props.anchor, peer), false)}
                >
                  Remove
                </button>
              </Show>
            </li>
          )}
        </For>
      </ul>
      <Show when={error()}>
        <div class="form-error" role="alert">{error()}</div>
      </Show>
      <div class="form-field-row">
        <button type="button" class="form-btn-secondary" onClick={() => props.onClose()}>
          Close
        </button>
        <button
          type="button"
          class="form-btn-secondary"
          disabled={busy()}
          onClick={() => void run(() => handleCancelCreation(props.anchor), true)}
        >
          Cancel creation
        </button>
      </div>
    </div>
  );
};

export default CoCreatorStatusView;
