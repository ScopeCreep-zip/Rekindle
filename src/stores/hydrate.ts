import { reconcile } from "solid-js/store";
import {
  handleLoadAutoModRules,
  handleResolveCommunityImageDataUrls,
} from "../actions/community.actions";
import { fetchAvatarUrl } from "../ipc/avatar";
import { commands } from "../ipc/commands";
import {
  foldFriendSnapshot,
  transformCommunityMap,
  transformExpression,
  transformMember,
} from "../utils/transformers";
import type { UserStatus } from "./auth.store";
import { setAuthState } from "./auth.store";
import { setCommunityMembers, setCommunityState } from "./community.store";
import { friendsState, setFriendsState } from "./friends.store";

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
      // Our status from the Host's own presence, the one source every
      // window reads (an auto-away or a pick made before this window
      // opened, or before a stream resync, is never reset to Online).
      const presence = await commands.getOwnPresence();
      setAuthState({
        isLoggedIn: true,
        publicKey: identity.publicKey,
        displayName: identity.displayName,
        avatarUrl,
        status: presence.status as UserStatus,
        statusMessage: presence.statusMessage || null,
      });
    }
  } catch (e) {
    console.error("Failed to hydrate identity:", e);
  }

  try {
    const friends = await commands.getFriends();
    setFriendsState("friends", reconcile(foldFriendSnapshot(friends, friendsState.friends)));
  } catch (e) {
    console.error("Failed to hydrate friends:", e);
  }

  try {
    const details = await commands.getCommunityDetails();
    setCommunityState("communities", transformCommunityMap(details));
    // Our ops not yet landed, from the Host (ADR 0013: a new window gets no
    // replay, so one opened after an op's `Pending` reads it here).
    setCommunityState(
      "opPublications",
      reconcile(Object.fromEntries(details.map((c) => [c.id, c.opPublications ?? {}])))
    );
    // Hydrate members for each community (fire-and-forget, parallel)
    for (const c of details) {
      commands
        .getCommunityMembers(c.id)
        .then((members) => {
          setCommunityMembers(c.id, members.map(transformMember));
        })
        .catch((e) => {
          console.error(`Failed to hydrate members for ${c.id}:`, e);
        });
      commands
        .listExpressions(c.id)
        .then((expressions) => {
          setCommunityState(
            "communities",
            c.id,
            "expressions",
            expressions.map(transformExpression)
          );
        })
        .catch((e) => {
          console.error(`Failed to hydrate expressions for ${c.id}:`, e);
        });
      void handleLoadAutoModRules(c.id);
      void handleResolveCommunityImageDataUrls(c.id);
    }
  } catch (e) {
    console.error("Failed to hydrate communities:", e);
  }
}
