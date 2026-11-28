import {
  type Component,
  createMemo,
  createResource,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import { handleRemoveFriend } from "../actions/buddy.actions";
import { handleStartDmCall } from "../actions/calls.actions";
import Avatar from "../components/common/Avatar";
import GameIcon from "../components/game/GameIcon";
import FriendNicknameField from "../components/profile/FriendNicknameField";
import SafetyNumberPanel from "../components/profile/SafetyNumberPanel";
import VerifiedBadge from "../components/profile/VerifiedBadge";
import StatusDot from "../components/status/StatusDot";
import Titlebar from "../components/titlebar/Titlebar";
import { subscribeProfilePresenceEvents } from "../handlers/presence-events.handlers";
import { useSafetyNumber } from "../hooks/useSafetyNumber";
import { ICON_ACCOUNT_REMOVE, ICON_PHONE, ICON_SEND, ICON_VIDEO } from "../icons";
import { startEventStream } from "../ipc/channels";
import { commands } from "../ipc/commands";
import { communityState } from "../stores/community.store";
import { friendsState } from "../stores/friends.store";
import { hydrateState } from "../stores/hydrate";
import { formatDuration, formatRelativeTime } from "../utils/formatting";
import { setWindowKind } from "../utils/window_kind";

function getKeyFromUrl(): string {
  const params = new URLSearchParams(window.location.search);
  return params.get("key") ?? "";
}

const ProfileWindow: Component = () => {
  const publicKey = getKeyFromUrl();
  const [confirmRemove, setConfirmRemove] = createSignal(false);
  const safety = useSafetyNumber(publicKey);
  // What the friend last shared (plan E7.2); empty when they share none.
  const [gameTotals] = createResource(() => commands.getFriendGameTotals(publicKey));
  // Their bio as they last sent it (plan E7.2), shown as plain text.
  const [bio] = createResource(() => commands.getFriendBio(publicKey));
  let unlistenPresence: Promise<import("@tauri-apps/api/event").UnlistenFn> | undefined;

  onMount(() => {
    hydrateState();
    unlistenPresence = subscribeProfilePresenceEvents(publicKey);
    void startEventStream(() => void hydrateState());
  });

  onCleanup(() => {
    unlistenPresence?.then((unlisten) => unlisten());
  });

  const friend = createMemo(() => {
    return friendsState.friends[publicKey];
  });

  const displayName = createMemo(() => {
    return friend()?.displayName ?? `${publicKey.slice(0, 16)}...`;
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

  setWindowKind("profile");
  return (
    <div class="app-frame">
      <Titlebar title={`Profile — ${displayName()}`} />
      <div class="profile-content">
        <Avatar displayName={displayName()} size={64} />
        <div class="profile-display-name">{displayName()}</div>
        <VerifiedBadge view={safety.view()} />
        <Show when={friend()}>
          <div class="profile-status-row">
            <StatusDot status={friend().status} />
            <span class="profile-status-label">{friend().status}</span>
          </div>
          <Show when={friend().statusMessage}>
            <div class="profile-status-message">{friend().statusMessage}</div>
          </Show>
        </Show>
        <div class="profile-section">
          <div class="profile-section-label">Public Key</div>
          <div class="profile-key-display">{publicKey}</div>
        </div>
        <Show when={friend()}>
          <div class="profile-section">
            <div class="profile-section-label">Safety Number</div>
            <SafetyNumberPanel peerKey={publicKey} view={safety.view()} onChanged={safety.reload} />
          </div>
        </Show>
        <Show when={friend()?.gameInfo}>
          <div class="profile-section">
            <div class="profile-section-label">Currently Playing</div>
            <div class="profile-game-name">
              <GameIcon iconKey={friend().gameInfo!.iconKey} />
              <span>{friend().gameInfo!.gameName}</span>
            </div>
            <Show when={friend().gameInfo!.serverAddress}>
              <div class="profile-game-server">on {friend().gameInfo!.serverAddress}</div>
            </Show>
            <Show when={friend().gameInfo!.startedAt}>
              <div class="profile-game-elapsed">{formatElapsed(friend().gameInfo!.startedAt!)}</div>
            </Show>
          </div>
        </Show>
        <Show when={bio()}>
          <div class="profile-section">
            <div class="profile-section-label">Bio</div>
            <p class="profile-bio">{bio()}</p>
          </div>
        </Show>
        <Show when={(gameTotals() ?? []).length > 0}>
          <div class="profile-section">
            <div class="profile-section-label">Game Time</div>
            <ul class="game-time-list" aria-label="Game time">
              <For each={gameTotals()}>
                {(game) => (
                  <li class="game-time-row">
                    <GameIcon iconKey={game.iconKey} />
                    <span class="game-time-name">{game.gameName}</span>
                    <span class="game-time-total">{formatDuration(game.totalSecs)}</span>
                    <span class="game-time-last">{formatRelativeTime(game.lastPlayed)}</span>
                  </li>
                )}
              </For>
            </ul>
          </div>
        </Show>
        <div class="profile-section">
          <div class="profile-section-label">Mutual Communities</div>
          <Show
            when={mutualCommunities().length > 0}
            fallback={<div class="profile-empty-hint">No mutual communities</div>}
          >
            <For each={mutualCommunities()}>
              {(community) => <div class="profile-mutual-item">{community.name}</div>}
            </For>
          </Show>
        </div>
        <Show when={friend()}>
          {(f) => <FriendNicknameField publicKey={publicKey} nickname={f().nickname ?? null} />}
        </Show>
        <Show when={friend()}>
          <div class="profile-actions">
            <button
              type="button"
              class="form-btn-secondary"
              onClick={() => commands.openChatWindow(publicKey)}
            >
              <span class="nf-icon">{ICON_SEND}</span> Send Message
            </button>
            {/* Wave 12 W12.8 — call buttons in profile header. */}
            <button
              type="button"
              class="form-btn-secondary"
              title="Start a voice call"
              onClick={() => void handleStartDmCall(publicKey, displayName(), false)}
            >
              <span class="nf-icon">{ICON_PHONE}</span> Voice Call
            </button>
            <button
              type="button"
              class="form-btn-secondary"
              title="Start a video call"
              onClick={() => void handleStartDmCall(publicKey, displayName(), true)}
            >
              <span class="nf-icon">{ICON_VIDEO}</span> Video Call
            </button>
            <Show when={!confirmRemove()}>
              <button type="button" class="form-btn-danger" onClick={() => setConfirmRemove(true)}>
                <span class="nf-icon">{ICON_ACCOUNT_REMOVE}</span> Remove Friend
              </button>
            </Show>
            <Show when={confirmRemove()}>
              <button
                type="button"
                class="form-btn-danger"
                onClick={() => handleRemoveFriend(publicKey)}
              >
                Confirm Remove
              </button>
              <button
                type="button"
                class="form-btn-secondary"
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
  return `Playing for ${formatDuration(seconds)}`;
}

export default ProfileWindow;
