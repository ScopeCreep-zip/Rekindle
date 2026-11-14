import { Component, For, Show } from "solid-js";
import { communityState } from "../../stores/community.store";

/**
 * Our governance changes the backend has not yet landed in this community
 * (`opPublication` events): how many are publishing, and each refused one
 * with the community's reason. No retry: a queued op lands by itself.
 */
const GovernancePublicationStatus: Component<{ communityId: string }> = (props) => {
  const states = () => Object.values(communityState.opPublications[props.communityId] ?? {});
  const pending = () => states().filter((s) => s.kind === "pending").length;
  const failures = () =>
    states().flatMap((s) => (s.kind === "failed" ? [s.reason] : []));
  return (
    <Show when={pending() > 0 || failures().length > 0}>
      <div class="governance-publication" role="status" aria-live="polite">
        <Show when={pending() > 0}>
          <div class="governance-publication-pending">
            {pending() === 1
              ? "Publishing 1 change; it applies once the network accepts it"
              : `Publishing ${pending()} changes; they apply once the network accepts them`}
          </div>
        </Show>
        <For each={failures()}>
          {(reason) => <div class="governance-publication-failed">Change not applied: {reason}</div>}
        </For>
      </div>
    </Show>
  );
};

export default GovernancePublicationStatus;
