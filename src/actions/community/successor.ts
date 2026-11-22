import { commands } from "../../ipc/commands";
import type { SuccessorOffer } from "../../ipc/commands/community";
import { markSuccessorFollowed } from "../../stores/successor.store";
import { errorMessage } from "../../utils/error";

// Successor communities (ADR 0011, plan E3.2 S7s.4). Every write is a
// `hostInvoke`: the Host's intent gate (ADR 0019) shows its review with the
// facts it states (the new creator set and who leaves it for a publish,
// the signer and both links for a follow), and a cancelled review rejects.
// The Host validates; the window shows its refusal as given.

/// The Host's successor snapshot for `community`, or null (logged) when it
/// is not read.
export async function handleGetCommunitySuccessors(community: string): Promise<SuccessorOffer[] | null> {
  try {
    return await commands.getCommunitySuccessors(community);
  } catch (e) {
    console.warn("successor snapshot not read:", errorMessage(e));
    return null;
  }
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
export async function handleFollowSuccessor(community: string, newAnchor: string): Promise<string | null> {
  try {
    await commands.followCommunitySuccessor(community, newAnchor);
    markSuccessorFollowed(newAnchor);
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}
