import { commands } from "../ipc/commands";
import { setAuthState } from "./auth.store";
import { fetchAvatarUrl } from "../ipc/avatar";
import { setFriendsState } from "./friends.store";
import { setCommunityState } from "./community.store";
import { transformExpression, transformFriendMap, transformCommunityMap, transformMember } from "../utils/transformers";
import {
  handleLoadAutoModRules,
  handleResolveCommunityImageDataUrls,
} from "../handlers/community.handlers";

/**
 * Hydrate frontend stores from the Rust backend.
 *
 * Lives in `stores/`, not `ipc/`. It reads through the IPC wrappers and
 * writes store state — which is the stores tier's job description, not
 * the Tauri boundary's. Sitting under `src/ipc/` made it four
 * `ipc-is-leaf` violations on its own, since a leaf may not reach up
 * into stores or handlers.
 *
 * Each Tauri webview has its own isolated JavaScript context,
 * so SolidJS stores are empty when a new window opens.
 * This function loads identity + friends + communities from the backend.
 */
export async function hydrateState(): Promise<void> {
  try {
    const identity = await commands.getIdentity();
    if (identity) {
      const avatarUrl = await fetchAvatarUrl(identity.publicKey);
      setAuthState({
        isLoggedIn: true,
        publicKey: identity.publicKey,
        displayName: identity.displayName,
        avatarUrl,
        status: "online",
      });
    }
  } catch (e) {
    console.error("Failed to hydrate identity:", e);
  }

  try {
    const friends = await commands.getFriends();
    setFriendsState("friends", transformFriendMap(friends));
  } catch (e) {
    console.error("Failed to hydrate friends:", e);
  }

  try {
    const details = await commands.getCommunityDetails();
    setCommunityState("communities", transformCommunityMap(details));
    // Hydrate members for each community (fire-and-forget, parallel)
    for (const c of details) {
      commands.getCommunityMembers(c.id).then((members) => {
        setCommunityState("communities", c.id, "members", members.map(transformMember));
      }).catch((e) => {
        console.error(`Failed to hydrate members for ${c.id}:`, e);
      });
      commands.listExpressions(c.id).then((expressions) => {
        setCommunityState("communities", c.id, "expressions", expressions.map(transformExpression));
      }).catch((e) => {
        console.error(`Failed to hydrate expressions for ${c.id}:`, e);
      });
      void handleLoadAutoModRules(c.id);
      void handleResolveCommunityImageDataUrls(c.id);
    }
  } catch (e) {
    console.error("Failed to hydrate communities:", e);
  }
}
