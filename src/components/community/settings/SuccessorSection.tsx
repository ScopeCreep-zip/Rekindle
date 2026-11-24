import { Component, For, Show, createEffect, createSignal } from "solid-js";
import { successorsOf, trackSuccessors } from "../../../stores/successor.store";
import { communityState } from "../../../stores/community.store";
import { handlePublishSuccessor } from "../../../actions/community/successor";
import FormField from "../../common/FormField";
import { deniedFor } from "../../../utils/denied";

interface SuccessorSectionProps {
  communityId: string;
}

/**
 * The community's successor (ADR 0011 "Successor community", plan E3.2
 * S7s.4): "Re-create community…" publishes a successor with us as its sole
 * creator and an invite sealed to every current member (lead decision
 * (34)), through the Host's Destructive intent review (ADR 0019), which
 * states the new creator set and who leaves it. Only a creator may: the
 * action is enabled while the Host states we hold a creator seat
 * (`weAreCreator`), and says why otherwise. Below it, the successors
 * the Host offered for this community (lead decision (33)), so a publish
 * shows as pending here once the Host announces it.
 */
const SuccessorSection: Component<SuccessorSectionProps> = (props) => {
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  // Offers made before this window opened (lead decision (33)).
  createEffect(() => trackSuccessors(props.communityId));
  const offers = () => successorsOf(props.communityId);
  // Never-hide: Re-create is drawn for everyone; the Host says why not.
  const denied = (): string | null => deniedFor(communityState.communities[props.communityId], "offerSuccessor");
  const reasonText = (reason: string): string => (reason === "transfer" ? "Handed over" : "Re-created");
  const heldText = (anchor: string): string =>
    communityState.communities[anchor] ? "you are a member" : "not joined here";

  async function publish(): Promise<void> {
    setBusy(true);
    setError(null);
    const err = await handlePublishSuccessor(props.communityId);
    setBusy(false);
    if (err) setError(err);
  }

  return (
    <FormField label="Successor">
      <div class="form-field-row">
        <button
          type="button"
          class="form-btn-danger"
          disabled={busy() || denied() !== null}
          title={denied() ?? undefined}
          aria-describedby={denied() === null ? undefined : "successor-denied"}
          aria-label="Re-create community under a new anchor and invite every member"
          onClick={() => void publish()}
        >
          {busy() ? "Re-creating..." : "Re-create community…"}
        </button>
      </div>
      <Show when={denied()}>
        {(why) => (
          <div class="settings-hint" id="successor-denied">
            {why()}
          </div>
        )}
      </Show>
      <Show when={error()}>
        <div class="form-error" role="alert">{error()}</div>
      </Show>
      <Show
        when={offers().length > 0}
        fallback={<div class="settings-hint">No successor has been offered.</div>}
      >
        <For each={offers()}>
          {(offer) => (
            <div class="settings-value" role="status">
              {reasonText(offer.reason)}: {offer.newAnchor.slice(0, 16)}… — signed by{" "}
              {offer.signer.slice(0, 12)}…, {heldText(offer.newAnchor)}
            </div>
          )}
        </For>
      </Show>
    </FormField>
  );
};

export default SuccessorSection;
