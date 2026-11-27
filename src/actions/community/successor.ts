import { eventStreamLive } from "../../ipc/channels";
import { commands } from "../../ipc/commands";
import type { SuccessorOffer } from "../../ipc/commands/community";
import {
  applySuccessorSnapshot,
  markSuccessorFollowed,
  successorSeq,
} from "../../stores/successor.store";
import { errorMessage } from "../../utils/error";

// Successor communities (ADR 0011, plan E3.2 S7s.4). Every write is a
// `hostInvoke`: the Host's intent gate (ADR 0019) shows its review with the
// facts it states (the new creator set and who leaves it for a publish,
// the signer and both links for a follow), and a cancelled review rejects.
// The Host validates; the window shows its refusal as given.

/// The successors the Host's snapshot (`CommunitySuccessors`) offers for
/// `community`, or null (logged) when it is not read. Its link requests
/// (SS4) are answered by the co-creator link view, not here.
export async function handleGetCommunitySuccessors(
  community: string
): Promise<SuccessorOffer[] | null> {
  try {
    return (await commands.getCommunitySuccessors(community)).offers;
  } catch (e) {
    console.warn("successor snapshot not read:", errorMessage(e));
    return null;
  }
}

const trackedSuccessors = new Set<string>();

/// Read `community`'s successor snapshot once this window's event stream is
/// live; once per community per window.
export function trackSuccessors(community: string): void {
  if (trackedSuccessors.has(community)) return;
  trackedSuccessors.add(community);
  void eventStreamLive().then(async () => {
    const before = successorSeq();
    const snapshot = await handleGetCommunitySuccessors(community);
    if (snapshot) applySuccessorSnapshot(snapshot, before);
  });
}

/// Re-create `community` under a new anchor with us as its sole creator,
/// sealing an invite to every current member (lead decision (34)); the
/// Host's error, or null. The offer arrives as `communitySuccessorOffered`.
export async function handlePublishSuccessor(community: string): Promise<string | null> {
  try {
    await commands.publishCommunitySuccessor(community);
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

/// Follow `newAnchor`, a successor of `community`, with the invite sealed to
/// us; the old community is kept. The Host's error, or null once followed
/// (the row then leaves).
export async function handleFollowSuccessor(
  community: string,
  newAnchor: string
): Promise<string | null> {
  try {
    await commands.followCommunitySuccessor(community, newAnchor);
    markSuccessorFollowed(newAnchor);
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}
