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
