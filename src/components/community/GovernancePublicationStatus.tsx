import { Component, For, Show } from "solid-js";
import type { OpQuarantineReason } from "../../ipc/channels/community_subscription_events";
import { communityState } from "../../stores/community.store";

const QUARANTINE_TEXT: Record<OpQuarantineReason, string> = {
  afterAuthorRemoval: "written after its author's removal",
  twoCheckpointEndorsementsInOneRound: "two checkpoint endorsements in one round",
  checkpointEquivocation: "checkpoint equivocation",
};

/**
 * Our governance changes the backend has not yet landed in this community
 * (`opPublication` events): how many are publishing, and each refused one
 * with the community's reason. No retry: a queued op lands by itself.
 * Below them, each governance op the backend set aside
 * (`governanceOpQuarantined`, plan E3.2 S7c.5) with its reason. First, when
 * the inviter's snapshot did not match the full history
 * (`checkpointSnapshotMismatch`, plan E3.2 S7c.8): the notice stays.
 */
const GovernancePublicationStatus: Component<{ communityId: string }> = (props) => {
  const states = () => Object.values(communityState.opPublications[props.communityId] ?? {});
  const pending = () => states().filter((s) => s.kind === "pending").length;
  const failures = () =>
    states().flatMap((s) => (s.kind === "failed" ? [s.reason] : []));
  const quarantined = () => Object.values(communityState.quarantinedOps[props.communityId] ?? {});
  const mismatch = () => communityState.snapshotMismatch[props.communityId] === true;
  const name = () => communityState.communities[props.communityId]?.name ?? "This community";
  return (
    <Show when={mismatch() || pending() > 0 || failures().length > 0 || quarantined().length > 0}>
      <div class="governance-publication" role="status" aria-live="polite">
        <Show when={mismatch()}>
          <div class="governance-publication-failed">
            The snapshot of {name()} your inviter's app sent did not match its full history.
            {" "}{name()} now uses the full history.
          </div>
        </Show>
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
        <For each={quarantined()}>
          {(op) => (
            <div class="governance-publication-failed">
              Change by {op.author.slice(0, 8)} set aside: {QUARANTINE_TEXT[op.reason]}
            </div>
          )}
        </For>
      </div>
    </Show>
  );
};

export default GovernancePublicationStatus;
