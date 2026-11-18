import { setCommunityState, communityState } from "../../stores/community.store";
import { commands } from "../../ipc/commands";
import { transformCommunityDetail, transformMember } from "../../utils/transformers";
import {
  handleLoadAutoModRules,
  handleLoadExpressions,
  handleResolveCommunityImageDataUrls,
} from "../../actions/community/lifecycle";

// An event can name a community this window does not hold yet (one just
// created or joined elsewhere: walk 13, M45/B61). Writing under its missing
// row throws; dropping the event loses the change. The window re-reads the
// community from the Host instead, as hydration does, and applies the event
// to the row it then holds. Reads for one community coalesce.
const reading = new Map<string, Promise<boolean>>();

/** Read `community` into the store; resolves whether the window holds it. */
function rereadCommunity(community: string): Promise<boolean> {
  const running = reading.get(community);
  if (running) return running;
  const read = commands
    .getCommunityDetails()
    .then(async (details) => {
      if (communityState.communities[community]) return true;
      const detail = details.find((d) => d.id === community);
      // Not one of ours: the event is not this window's to apply.
      if (!detail) return false;
      setCommunityState("communities", community, transformCommunityDetail(detail));
      const members = await commands.getCommunityMembers(community);
      setCommunityState("communities", community, "members", members.map(transformMember));
      void handleLoadExpressions(community);
      void handleLoadAutoModRules(community);
      void handleResolveCommunityImageDataUrls(community);
      return true;
    })
    .catch((e) => {
      console.error(`Failed to read community ${community}:`, e);
      return communityState.communities[community] !== undefined;
    })
    .finally(() => reading.delete(community));
  reading.set(community, read);
  return read;
}

/**
 * Run `apply` against `community`'s store row: at once when the window holds
 * it, else after re-reading it from the Host (never when the Host does not
 * list it).
 */
export function withCommunity(community: string, apply: () => void): void {
  if (communityState.communities[community]) {
    apply();
    return;
  }
  void rereadCommunity(community).then((held) => {
    if (held) apply();
  });
}
