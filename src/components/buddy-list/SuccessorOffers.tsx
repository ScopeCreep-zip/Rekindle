import { type Component, For, Show, createEffect, createSignal } from "solid-js";
import { openSuccessorOffers } from "../../stores/successor.store";
import { communityState } from "../../stores/community.store";
import { handleFollowSuccessor, trackSuccessors } from "../../actions/community/successor";

/**
 * Successor communities the Host offered (ADR 0011, plan E3.2 S7s.4, lead
 * decision (33)), beside the co-creator invites: "<community> moved to a
 * new community" with Follow, which goes through the Host's intent review
 * (ADR 0019; it states the signer and that both communities link). A row
 * leaves once followed. The Host decides who may follow; its refusal (a
 * member it did not continue) is shown as given.
 */
const SuccessorOffers: Component = () => {
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  // Offers made before this window opened, per community we hold.
  createEffect(() => {
    for (const id of Object.keys(communityState.communities)) trackSuccessors(id);
  });
  // A successor we already hold (we published it, or followed it from
  // another window) has nothing to follow.
  const offers = () =>
    openSuccessorOffers().filter((offer) => communityState.communities[offer.newAnchor] === undefined);
  const nameOf = (community: string): string =>
    communityState.communities[community]?.name ?? `${community.slice(0, 12)}…`;
  const movedText = (reason: string): string =>
    reason === "transfer" ? "handed over to a new community" : "moved to a new community";

  async function follow(community: string, newAnchor: string): Promise<void> {
    setBusy(true);
    setError(null);
    const err = await handleFollowSuccessor(community, newAnchor);
    setBusy(false);
    if (err) setError(err);
  }

  return (
    <Show when={offers().length > 0}>
      <div class="pending-section">
        <div class="pending-header">Moved Communities ({offers().length})</div>
        <Show when={error()}>
          <div class="form-error" role="alert">{error()}</div>
        </Show>
        <For each={offers()}>
          {(offer) => (
            <div class="pending-item">
              <div class="pending-info">
                <span class="buddy-name">{nameOf(offer.community)}</span>
                <span class="pending-message">{movedText(offer.reason)}</span>
              </div>
              <button
                type="button"
                class="pending-btn-accept"
                disabled={busy()}
                aria-label={`Follow ${nameOf(offer.community)} to its new community`}
                onClick={() => void follow(offer.community, offer.newAnchor)}
              >
                Follow
              </button>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
};

export default SuccessorOffers;
