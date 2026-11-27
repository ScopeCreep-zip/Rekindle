/**
 * Successor communities (ADR 0011 "Successor community", plan E3.2 S7s.3e,
 * lead decision (33)): the successors the Host offered for each community
 * we hold. Fed only by the Host's `communitySuccessorOffered` governance
 * event; the window decides nothing, it renders what the Host said. A
 * window opened after the offer reads the Host's snapshot
 * (`CommunitySuccessors`, per community) once its event stream is live
 * (plan VI1's pattern); an offer an event stated while the snapshot was in
 * flight wins over the snapshot's copy of it.
 *
 * The one piece of window state here is `followed`: a successor this
 * window followed leaves its row (as an answered co-creator invite does).
 */

import { createStore, produce } from "solid-js/store";
import type { GovernanceEvent } from "../ipc/channels/community_subscription_events";
import type { SuccessorOffer } from "../ipc/commands/community";

interface SuccessorState {
  /** Per community, its offered successors by new anchor. */
  offers: Record<string, Record<string, SuccessorOffer>>;
  /** Successors this window followed, by new anchor. */
  followed: Record<string, true>;
}

const [successorState, setSuccessorState] = createStore<SuccessorState>({
  offers: {},
  followed: {},
});

/**
 * When each offer was last set by an event, on one counter: an event
 * carries the offer whole and the stream delivers in order, so an offer an
 * event set while the snapshot was in flight is never older than the
 * snapshot's, and the snapshot leaves it alone (plan VI1).
 */
let seq = 0;
const setBy = new Map<string, number>();
const offerKey = (community: string, newAnchor: string): string => `${community}:${newAnchor}`;

function putOffer(offer: SuccessorOffer): void {
  setSuccessorState(
    "offers",
    produce((all) => {
      all[offer.community] = { ...(all[offer.community] ?? {}), [offer.newAnchor]: offer };
    })
  );
}

/** Apply a governance event; whether it was a successor offer. */
export function applySuccessorEvent(g: GovernanceEvent): boolean {
  if (!("communitySuccessorOffered" in g)) return false;
  const offer = g.communitySuccessorOffered;
  seq += 1;
  setBy.set(offerKey(offer.community, offer.newAnchor), seq);
  putOffer(offer);
  return true;
}

/** The event sequence number now; a snapshot read after it yields to any
 *  offer an event set since. */
export function successorSeq(): number {
  return seq;
}

/** Apply a successor snapshot read after `before`: an offer an event set
 *  since then is the newer statement and is kept. */
export function applySuccessorSnapshot(snapshot: SuccessorOffer[], before: number): void {
  for (const offer of snapshot) {
    if ((setBy.get(offerKey(offer.community, offer.newAnchor)) ?? 0) <= before) putOffer(offer);
  }
}

/** Mark a successor this window followed; its row leaves. */
export function markSuccessorFollowed(newAnchor: string): void {
  setSuccessorState("followed", newAnchor, true);
}

/** Every offered successor not yet followed here, across communities. */
export function openSuccessorOffers(): SuccessorOffer[] {
  return Object.values(successorState.offers)
    .flatMap((byAnchor) => Object.values(byAnchor))
    .filter((offer) => !successorState.followed[offer.newAnchor]);
}

/** The successors offered for `community`. */
export function successorsOf(community: string): SuccessorOffer[] {
  return Object.values(successorState.offers[community] ?? {});
}
