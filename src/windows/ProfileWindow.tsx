import { Component, createMemo, createSignal, For, onMount, onCleanup, Show } from "solid-js";
import Titlebar from "../components/titlebar/Titlebar";
import Avatar from "../components/common/Avatar";
import StatusDot from "../components/status/StatusDot";
import { friendsState } from "../stores/friends.store";
import { communityState } from "../stores/community.store";
import { subscribeProfilePresenceEvents } from "../handlers/presence-events.handlers";
import { hydrateState } from "../ipc/hydrate";
import { commands } from "../ipc/commands";
import { handleRemoveFriend } from "../handlers/buddy.handlers";
import { ICON_SEND, ICON_ACCOUNT_REMOVE } from "../icons";

function getKeyFromUrl(): string {
  const params = new URLSearchParams(window.location.search);
  return params.get("key") ?? "";
}

const ProfileWindow: Component = () => {
  const publicKey = getKeyFromUrl();
  const [confirmRemove, setConfirmRemove] = createSignal(false);
  let unlistenPresence: Promise<import("@tauri-apps/api/event").UnlistenFn> | undefined;

  onMount(() => {
    hydrateState();
    unlistenPresence = subscribeProfilePresenceEvents(publicKey);
  });

  onCleanup(() => {
    unlistenPresence?.then((unlisten) => unlisten());
  });

  const friend = createMemo(() => {
    return friendsState.friends[publicKey];
  });

  const displayName = createMemo(() => {
    return friend()?.displayName ?? publicKey.slice(0, 16) + "...";
  });

  const mutualCommunities = createMemo(() => {
    const result: { id: string; name: string }[] = [];
    for (const [id, community] of Object.entries(communityState.communities)) {
      const isMember = community.members.some((m) => m.pseudonymKey === publicKey);
      if (isMember) {
        result.push({ id, name: community.name });
      }
    }
    return result;
  });

  return (
    <div class="app-frame">
      <Titlebar title={`Profile — ${displayName()}`} />
      <div class="profile-content">
        <Avatar displayName={displayName()} size={64} />
        <div class="profile-display-name">{displayName()}</div>
        <Show when={friend()}>
          <div class="profile-status-row">
            <StatusDot status={friend()!.status} />
            <span class="profile-status-label">{friend()!.status}</span>
          </div>
          <Show when={friend()!.statusMessage}>
            <div class="profile-status-message">{friend()!.statusMessage}</div>
          </Show>
        </Show>
        <div class="profile-section">
          <div class="profile-section-label">Public Key</div>
          <div class="profile-key-display">{publicKey}</div>
        </div>
        <Show when={friend()?.gameInfo}>
          <div class="profile-section">
            <div class="profile-section-label">Currently Playing</div>
            <div class="profile-game-name">{friend()!.gameInfo!.gameName}</div>
            <Show when={friend()!.gameInfo!.startedAt}>
              <div class="profile-game-elapsed">
                {formatElapsed(friend()!.gameInfo!.startedAt!)}
              </div>
            </Show>
          </div>
        </Show>
        <div class="profile-section">
          <div class="profile-section-label">Mutual Communities</div>
          <Show when={mutualCommunities().length > 0} fallback={
            <div class="profile-empty-hint">No mutual communities</div>
          }>
            <For each={mutualCommunities()}>
              {(community) => (
                <div class="profile-mutual-item">{community.name}</div>
              )}
            </For>
          </Show>
        </div>
        <Show when={friend()}>
          <div class="profile-actions">
            <button
              class="profile-btn-message"
              onClick={() => commands.openChatWindow(publicKey, displayName())}
            >
              <span class="nf-icon">{ICON_SEND}</span> Send Message
            </button>
            <Show when={!confirmRemove()}>
              <button
                class="profile-btn-remove"
                onClick={() => setConfirmRemove(true)}
              >
                <span class="nf-icon">{ICON_ACCOUNT_REMOVE}</span> Remove Friend
              </button>
            </Show>
            <Show when={confirmRemove()}>
              <button
                class="profile-btn-remove profile-btn-confirm"
                onClick={() => handleRemoveFriend(publicKey)}
              >
                Confirm Remove
              </button>
              <button
                class="profile-btn-message"
                onClick={() => setConfirmRemove(false)}
              >
                Cancel
              </button>
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
};

function formatElapsed(seconds: number): string {
  const hours = Math.floor(seconds / 3600);
  const mins = Math.floor((seconds % 3600) / 60);
  if (hours > 0) {
    return `Playing for ${hours}h ${mins}m`;
  }
  return `Playing for ${mins}m`;
}

export default ProfileWindow;
