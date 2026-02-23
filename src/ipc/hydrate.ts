import { commands } from "./commands";
import { setAuthState } from "../stores/auth.store";
import { fetchAvatarUrl } from "./avatar";
import { setFriendsState } from "../stores/friends.store";
import { setCommunityState } from "../stores/community.store";
import { transformFriendMap, transformCommunityMap } from "../utils/transformers";

/**
 * Hydrate frontend stores from the Rust backend.
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
  } catch (e) {
    console.error("Failed to hydrate communities:", e);
  }
}
