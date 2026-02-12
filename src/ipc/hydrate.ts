import { commands } from "./commands";
import { setAuthState } from "../stores/auth.store";
import { fetchAvatarUrl } from "./avatar";
import { setFriendsState } from "../stores/friends.store";
import { setCommunityState } from "../stores/community.store";
import type { Friend } from "../stores/friends.store";
import type { Community } from "../stores/community.store";

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
    const friendMap: Record<string, Friend> = {};
    for (const f of friends) {
      friendMap[f.publicKey] = {
        publicKey: f.publicKey,
        displayName: f.displayName,
        nickname: f.nickname,
        status: (f.status as Friend["status"]) ?? "offline",
        statusMessage: f.statusMessage ?? null,
        gameInfo: f.gameInfo
          ? { gameName: f.gameInfo.gameName, gameId: f.gameInfo.gameId, startedAt: null }
          : null,
        group: f.group ?? "Friends",
        unreadCount: f.unreadCount,
        lastSeenAt: f.lastSeenAt ?? null,
        voiceChannel: null,
      };
    }
    setFriendsState("friends", friendMap);
  } catch (e) {
    console.error("Failed to hydrate friends:", e);
  }

  try {
    const details = await commands.getCommunityDetails();
    const communityMap: Record<string, Community> = {};
    for (const c of details) {
      communityMap[c.id] = {
        id: c.id,
        name: c.name,
        channels: c.channels.map((ch) => ({
          id: ch.id,
          name: ch.name,
          type: ch.channelType as "text" | "voice",
          unreadCount: ch.unreadCount,
        })),
        members: [],
        roles: [],
      };
    }
    setCommunityState("communities", communityMap);
  } catch (e) {
    console.error("Failed to hydrate communities:", e);
  }
}
