import { Component, For, Show, createSignal } from "solid-js";
import { friendsState } from "../../stores/friends.store";
import {
  handleAcceptRequest,
  handleRejectRequest,
} from "../../handlers/buddy.handlers";

const PendingRequests: Component = () => {
  const [error, setError] = createSignal<string | null>(null);

  async function onAccept(publicKey: string): Promise<void> {
    setError(null);
    const err = await handleAcceptRequest(publicKey);
    if (err) setError(err);
  }

  async function onReject(publicKey: string): Promise<void> {
    setError(null);
    const err = await handleRejectRequest(publicKey);
    if (err) setError(err);
  }

  return (
    <Show when={friendsState.pendingRequests.length > 0}>
      <div class="pending-section">
        <div class="pending-header">
          Pending Requests ({friendsState.pendingRequests.length})
        </div>
        <Show when={error()}>
          <div class="login-error">{error()}</div>
        </Show>
        <For each={friendsState.pendingRequests}>
          {(request) => (
            <div class="pending-item">
              <div class="pending-info">
                <span class="buddy-name">
                  {request.displayName || request.publicKey.slice(0, 12) + "..."}
                </span>
                <Show when={request.message}>
                  <span class="pending-message">{request.message}</span>
                </Show>
              </div>
              <button
                class="pending-btn-accept"
                onClick={() => onAccept(request.publicKey)}
              >
                Accept
              </button>
              <button
                class="pending-btn-reject"
                onClick={() => onReject(request.publicKey)}
              >
                Reject
              </button>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
};

export default PendingRequests;
