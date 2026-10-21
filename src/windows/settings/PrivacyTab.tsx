import { Component, createSignal, onCleanup, onMount, For, Show } from "solid-js";
import { subscribeChatEvents } from "../../ipc/channels";
import FormField from "../../components/common/FormField";
import RelaySettingsSection from "../../components/settings/RelaySettingsSection";
import { authState } from "../../stores/auth.store";
import { commands } from "../../ipc/commands";
import { handleUnblockUser } from "../../actions/buddy.actions";

const PrivacyTab: Component = () => {
  const [blockedUsers, setBlockedUsers] = createSignal<{ publicKey: string; displayName: string; blockedAt: number }[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [readReceipts, setReadReceipts] = createSignal(false);

  onMount(() => {
    commands.getPreferences().then((p) => setReadReceipts(p.readReceiptsEnabled)).catch((e) => {
      console.error("Failed to load the read-receipt setting:", e);
    });
  });

  // The Host reviews the change (a trust-changing intent); the switch shows
  // what it stored, or stays as it was when the change is refused.
  async function handleToggleReadReceipts(): Promise<void> {
    setError(null);
    try {
      const prefs = await commands.setReadReceiptsEnabled(!readReceipts());
      setReadReceipts(prefs.readReceiptsEnabled);
    } catch (e) {
      setError(`Read receipts not changed: ${String(e)}`);
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
          checked={readReceipts()}
          onChange={() => void handleToggleReadReceipts()}
        />
        <span class="buddy-name">Read receipts</span>
      </label>
      <div class="settings-hint">
        Friends see that you read their messages only when both of you turn
        read receipts on. They are never sent while you appear Invisible.
      </div>
      <div class="settings-section-title">Identity</div>
      <div class="form-field-row">
        <button class="form-btn-secondary" disabled>Export Identity</button>
        <button class="form-btn-secondary" disabled>Import Identity</button>
      </div>
      <div class="settings-hint">Identity export/import requires Stronghold integration.</div>
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
