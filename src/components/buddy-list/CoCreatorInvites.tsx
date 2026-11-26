import { Component, For, Show, createSignal } from "solid-js";
import { coCreatorState } from "../../stores/co-creator.store";
import { friendsState } from "../../stores/friends.store";
import { handleAnswerCreatorInvite, trackCoCreation } from "../../actions/community/co_create";

/**
 * Co-creator invites a friend sent us (plan E3.2 S7s.3f), beside the
 * friend requests in the buddy list's pending area: inviter and community
 * name, Accept / Decline through the Host's intent review (ADR 0019). A
 * row leaves once answered or when the Host says the inviter withdrew it.
 */
const CoCreatorInvites: Component = () => {
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  // Invites held before this window opened (plan E3.2 S7s.3g).
  trackCoCreation();
  const invites = () => Object.values(coCreatorState.invites);
  const inviterName = (peer: string): string =>
    friendsState.friends[peer]?.displayName ?? `${peer.slice(0, 12)}…`;

  async function answer(anchor: string, accept: boolean): Promise<void> {
    setBusy(true);
    setError(null);
    const err = await handleAnswerCreatorInvite(anchor, accept);
    setBusy(false);
    if (err) setError(err);
  }

  return (
    <Show when={invites().length > 0}>
      <div class="pending-section">
        <div class="pending-header">Co-creator Invites ({invites().length})</div>
        <Show when={error()}>
          <div class="form-error" role="alert">{error()}</div>
        </Show>
        <For each={invites()}>
          {(invite) => (
            <div class="pending-item">
              <div class="pending-info">
                <span class="buddy-name">{inviterName(invite.peerKey)}</span>
                <span class="pending-message">Co-create {invite.communityName}</span>
              </div>
              <button
                type="button"
                class="pending-btn-accept"
                disabled={busy()}
                aria-label={`Accept co-creating ${invite.communityName}`}
                onClick={() => void answer(invite.anchor, true)}
              >
                Accept
              </button>
              <button
                type="button"
                class="pending-btn-reject"
                disabled={busy()}
                aria-label={`Decline co-creating ${invite.communityName}`}
                onClick={() => void answer(invite.anchor, false)}
              >
                Decline
              </button>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
};

export default CoCreatorInvites;
