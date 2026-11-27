import { type Component, For, Show } from "solid-js";
import type { OpQuarantineReason } from "../../ipc/channels/community_subscription_events";
import { communityState } from "../../stores/community.store";

const QUARANTINE_TEXT: Record<OpQuarantineReason, string> = {
  afterAuthorRemoval: "written after its author's removal",
  twoCheckpointEndorsementsInOneRound: "two checkpoint endorsements in one round",
  recordClaimed: "member record already claimed by another member",
  authorityQuarantined: "quarantined because the change its authority came from was",
  deviceKeyRetired: "its device key was retired or forked before it",
  deviceKeyInert: "its device key was endorsed past six devices",
  deviceKeyOverCap: "its device key is not among its author's first six",
  deviceKeyNeedsReadmit: "its device key answers a fork with no re-admission",
  deviceKeyRecordTaken: "its device key names another device's record",
  checkpointEquivocation: "checkpoint equivocation",
  unknownOpKind: "a kind of change this version of Rekindle cannot read",
  unknownOpVersion: "a change from a newer version of Rekindle",
};

/** Reasons that quarantine the whole community, not one change (D15). */
const quarantinesCommunity = (reason: OpQuarantineReason) =>
  reason === "unknownOpKind" || reason === "unknownOpVersion";

/**
 * Our governance changes the backend has not yet landed in this community
 * (`opPublication` events): how many are publishing, and each refused one
 * with the community's reason. No retry: a queued op lands by itself.
 * Below them, each governance op the backend set aside
 * (`governanceOpQuarantined`, plan E3.2 S7c.5) with its reason. First, when
 * the inviter's snapshot did not match the full history
 * (`checkpointSnapshotMismatch`, plan E3.2 S7c.8): the notice stays. A
 * member's change this version cannot read quarantines the community (D15):
 * one notice says so, with the reason, above the rest.
 */
const GovernancePublicationStatus: Component<{ communityId: string }> = (props) => {
  const states = () => Object.values(communityState.opPublications[props.communityId] ?? {});
  const pending = () => states().filter((s) => s.kind === "pending").length;
  const failures = () => states().flatMap((s) => (s.kind === "failed" ? [s.reason] : []));
  const held = () => Object.values(communityState.quarantinedOps[props.communityId] ?? {});
  const quarantined = () => held().filter((op) => !quarantinesCommunity(op.reason));
  const unreadable = () => held().filter((op) => quarantinesCommunity(op.reason));
  const mismatch = () => communityState.snapshotMismatch[props.communityId] === true;
  const name = () => communityState.communities[props.communityId]?.name ?? "This community";
  const authorName = (pseudonym: string) =>
    communityState.communities[props.communityId]?.members.find(
      (member) => member.pseudonymKey === pseudonym
    )?.displayName ?? "a member";
  return (
    <Show when={mismatch() || pending() > 0 || failures().length > 0 || held().length > 0}>
      <div class="governance-publication" role="status" aria-live="polite">
        <Show when={unreadable()[0]}>
          {(op) => (
            <div class="governance-publication-failed">
              {name()} is quarantined: a change by {authorName(op().author)} is{" "}
              {QUARANTINE_TEXT[op().reason]}. Update Rekindle to apply it; until then {name()} shows
              its state without that change.
            </div>
          )}
        </Show>
        <Show when={mismatch()}>
          <div class="governance-publication-failed">
            The snapshot of {name()} your inviter's app sent did not match its full history.{" "}
            {name()} now uses the full history.
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
          {(reason) => (
            <div class="governance-publication-failed">Change not applied: {reason}</div>
          )}
        </For>
        <For each={quarantined()}>
          {(op) => (
            <div class="governance-publication-failed">
              Change by {authorName(op.author)} set aside: {QUARANTINE_TEXT[op.reason]}
            </div>
          )}
        </For>
      </div>
    </Show>
  );
};

export default GovernancePublicationStatus;
