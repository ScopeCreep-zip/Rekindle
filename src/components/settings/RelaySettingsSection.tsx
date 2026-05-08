import { Component, For, Show, createMemo, onMount } from "solid-js";
import { friendsState } from "../../stores/friends.store";
import {
  handleHydrateRelayState,
  handleRevokeRelay,
  handleVolunteerRelay,
  relayState,
} from "../../handlers/relay.handlers";
import { handleSaveSettings } from "../../handlers/settings.handlers";
import { settingsState } from "../../stores/settings.store";

/// Strand Relay Network (architecture §13) configuration block: shows
/// which friends we relay for, which friends offer to relay for us, and
/// lets the user toggle volunteering per friend.
const RelaySettingsSection: Component = () => {
  onMount(() => {
    handleHydrateRelayState();
  });

  const toggleAutoVolunteer = (): void => {
    void handleSaveSettings({
      autoVolunteerRelayForNewFriends:
        !settingsState.autoVolunteerRelayForNewFriends,
    });
  };

  const volunteeredKeys = createMemo(() => Object.keys(relayState.volunteeredFor));
  const receivedKeys = createMemo(() => Object.keys(relayState.receivedOffersFrom));
  const otherFriends = createMemo(() => {
    const all = Object.keys(friendsState.friends);
    return all.filter((k) => !relayState.volunteeredFor[k]);
  });

  function displayName(publicKey: string): string {
    const friend = friendsState.friends[publicKey];
    return friend?.displayName ?? `${publicKey.slice(0, 12)}…`;
  }

  return (
    <>
      <div class="settings-section-title">Strand Relay Network</div>
      <div class="settings-hint">
        Friends you relay for receive a dedicated route from your client. Other
        peers may use that route as a fallback when they can't reach your
        friend directly.
      </div>

      {/* W11.3 — explicit-consent toggle: when ON, accepting a new
          friend request also volunteers a relay route for that
          friend. OFF by default; per-friend never network-wide. */}
      <label class="settings-toggle-row">
        <input
          type="checkbox"
          checked={settingsState.autoVolunteerRelayForNewFriends}
          onChange={toggleAutoVolunteer}
        />
        <span class="settings-toggle-label">
          Auto-volunteer to relay for new friends
        </span>
        <div class="settings-hint">
          Only volunteers for friends you accept after enabling this. Existing
          friends are unchanged — opt them in via "Volunteer for a friend"
          below.
        </div>
      </label>

      <div class="relay-section-subtitle">Friends I relay for</div>
      <Show
        when={volunteeredKeys().length > 0}
        fallback={<div class="settings-hint">No active relay volunteer offers.</div>}
      >
        <For each={volunteeredKeys()}>
          {(key) => (
            <div class="relay-row">
              <span class="buddy-name">{displayName(key)}</span>
              <button
                class="form-btn-secondary"
                onClick={() => handleRevokeRelay(key)}
              >
                Stop relaying
              </button>
            </div>
          )}
        </For>
      </Show>

      <div class="relay-section-subtitle">Friends who relay for me</div>
      <Show
        when={receivedKeys().length > 0}
        fallback={<div class="settings-hint">No friends are relaying for you yet.</div>}
      >
        <For each={receivedKeys()}>
          {(key) => (
            <div class="relay-row">
              <span class="buddy-name">{displayName(key)}</span>
              <span class="relay-row-badge">active</span>
            </div>
          )}
        </For>
      </Show>

      <div class="relay-section-subtitle">Volunteer for a friend</div>
      <Show
        when={otherFriends().length > 0}
        fallback={<div class="settings-hint">All friends are already configured.</div>}
      >
        <For each={otherFriends()}>
          {(key) => (
            <div class="relay-row">
              <span class="buddy-name">{displayName(key)}</span>
              <button
                class="form-btn-primary"
                onClick={() => handleVolunteerRelay(key)}
              >
                Volunteer
              </button>
            </div>
          )}
        </For>
      </Show>
    </>
  );
};

export default RelaySettingsSection;
