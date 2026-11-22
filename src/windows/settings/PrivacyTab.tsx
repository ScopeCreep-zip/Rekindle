import { Component, createSignal, onCleanup, onMount, For, Show } from "solid-js";
import { subscribeChatEvents } from "../../ipc/channels";
import FormField from "../../components/common/FormField";
import RelaySettingsSection from "../../components/settings/RelaySettingsSection";
import { authState } from "../../stores/auth.store";
import { commands } from "../../ipc/commands";
import { handleUnblockUser } from "../../actions/buddy.actions";
import HiddenGamesSection from "../../components/settings/HiddenGamesSection";
import QuickWipeSection from "../../components/settings/QuickWipeSection";
import AuditLogSection from "../../components/settings/AuditLogSection";

const PrivacyTab: Component = () => {
  const [blockedUsers, setBlockedUsers] = createSignal<{ publicKey: string; displayName: string; blockedAt: number }[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  // `null` until the stored value loads: the box is unticked and disabled,
  // never showing a value it has not read (walk row ST28 read the initial
  // `false` back after reopening Settings, before the load answered).
  const [readReceipts, setReadReceipts] = createSignal<boolean | null>(null);
  const [shareGameTotals, setShareGameTotals] = createSignal<boolean | null>(null);
  const [shareCurrentGame, setShareCurrentGame] = createSignal<boolean | null>(null);
  const [keepActivity, setKeepActivity] = createSignal<boolean | null>(null);

  onMount(() => {
    commands.getPreferences().then((p) => {
      setReadReceipts(p.readReceiptsEnabled);
      setShareGameTotals(p.shareGameTotals);
      setShareCurrentGame(p.shareCurrentGame);
      setKeepActivity(p.keepActivityLog);
    }).catch((e) => {
      console.error("Failed to load the privacy settings:", e);
    });
  });

  // The Host reviews the change (a trust-changing intent); the switch shows
  // what it stored, or stays as it was when the change is refused.
  async function handleToggleReadReceipts(): Promise<void> {
    const current = readReceipts();
    if (current === null) return;
    setError(null);
    try {
      const prefs = await commands.setReadReceiptsEnabled(!current);
      setReadReceipts(prefs.readReceiptsEnabled);
    } catch (e) {
      setError(`Read receipts not changed: ${String(e)}`);
    }
  }

  // The same review; turning it on sends our totals to every friend.
  async function handleToggleShareGameTotals(): Promise<void> {
    const current = shareGameTotals();
    if (current === null) return;
    setError(null);
    try {
      const prefs = await commands.setShareGameTotals(!current);
      setShareGameTotals(prefs.shareGameTotals);
    } catch (e) {
      setError(`Game time sharing not changed: ${String(e)}`);
    }
  }

  // The same review; on, friends see the game we play (never a hidden one).
  async function handleToggleShareCurrentGame(): Promise<void> {
    const current = shareCurrentGame();
    if (current === null) return;
    setError(null);
    try {
      const prefs = await commands.setShareCurrentGame(!current);
      setShareCurrentGame(prefs.shareCurrentGame);
    } catch (e) {
      setError(`Current game sharing not changed: ${String(e)}`);
    }
  }

  // The Host's review; off clears the feed and writes nothing until on again.
  async function handleToggleKeepActivity(): Promise<void> {
    const current = keepActivity();
    if (current === null) return;
    setError(null);
    try {
      const prefs = await commands.setKeepActivityLog(!current);
      setKeepActivity(prefs.keepActivityLog);
    } catch (e) {
      setError(`Activity feed not changed: ${String(e)}`);
    }
  }

  const loadBlocked = (): void => {
    commands.getBlockedUsers().then(setBlockedUsers).catch((e) => {
      console.error("Failed to load blocked users:", e);
    });
  };
  // The Host's `blockChanged` (from any window or frontend) re-reads the list.
  const unsubscribe = subscribeChatEvents((event) => {
    if ("friend" in event && "blockChanged" in event.friend) loadBlocked();
  });
  onMount(loadBlocked);
  onCleanup(() => void unsubscribe.then((un) => un()));

  // Delegates to the shared action (`actions/buddy.actions.ts`) rather than
  // calling `commands.unblockUser` directly here a second time — this file
  // used to reimplement it locally with weaker error handling
  // (`console.error` only, nothing surfaced to the user).
  async function handleUnblock(publicKey: string): Promise<void> {
    setError(null);
    const err = await handleUnblockUser(publicKey);
    if (err) {
      setError(err);
      return;
    }
  }

  return (
    <>
      <div class="settings-section-title">Privacy</div>
      <FormField label="Public Key">
        <div class="profile-key-display">{authState.publicKey ?? "Not logged in"}</div>
      </FormField>
      <div class="settings-section-title">Read Receipts</div>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={readReceipts() === true}
          disabled={readReceipts() === null}
          onChange={() => void handleToggleReadReceipts()}
        />
        <span class="buddy-name">Read receipts</span>
      </label>
      <div class="settings-hint">
        Friends see that you read their messages only when both of you turn
        read receipts on. They are never sent while you appear Invisible.
      </div>
      <div class="settings-section-title">Current Game</div>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={shareCurrentGame() === true}
          disabled={shareCurrentGame() === null}
          onChange={() => void handleToggleShareCurrentGame()}
        />
        <span class="buddy-name">Show friends the game I'm playing</span>
      </label>
      <div class="settings-hint">
        Off, games are still detected for your own game time, but friends
        never see what you play. Hidden games are never shown either way.
      </div>
      <HiddenGamesSection />
      <div class="settings-section-title">Game Time</div>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={shareGameTotals() === true}
          disabled={shareGameTotals() === null}
          onChange={() => void handleToggleShareGameTotals()}
        />
        <span class="buddy-name">Share game time with friends</span>
      </label>
      <div class="settings-hint">
        Friends see how long you have played each game, sent to them
        end-to-end encrypted. Turning it off stops new totals; friends keep
        what they last received until you are no longer friends.
      </div>
      <div class="settings-section-title">Activity Feed</div>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={keepActivity() === true}
          disabled={keepActivity() === null}
          onChange={() => void handleToggleKeepActivity()}
        />
        <span class="buddy-name">Keep an activity feed of my friends</span>
      </label>
      <div class="settings-hint">
        Your friends' games, name and status changes, kept on this device
        only and never sent anywhere. Turning it off clears the feed.
      </div>
      <div class="settings-section-title">Identity</div>
      <div class="form-field-row">
        <button class="form-btn-secondary" disabled>Export Identity</button>
        <button class="form-btn-secondary" disabled>Import Identity</button>
      </div>
      <div class="settings-hint">Identity export/import requires Stronghold integration.</div>
      <QuickWipeSection />
      <AuditLogSection />
      <div class="settings-section-title">Blocked Users</div>
      <Show when={error()}>
        <div class="form-error">{error()}</div>
      </Show>
      <Show when={blockedUsers().length > 0} fallback={
        <div class="settings-hint">No blocked users.</div>
      }>
        <For each={blockedUsers()}>
          {(user) => (
            <div class="blocked-user-item">
              <span class="buddy-name">{user.displayName || user.publicKey.slice(0, 12) + "..."}</span>
              <button class="form-btn-secondary" onClick={() => handleUnblock(user.publicKey)}>
                Unblock
              </button>
            </div>
          )}
        </For>
      </Show>
      <RelaySettingsSection />
    </>
  );
};

export default PrivacyTab;
