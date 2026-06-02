import { Component, createSignal, onMount, For, Show } from "solid-js";
import FormField from "../../components/common/FormField";
import RelaySettingsSection from "../../components/settings/RelaySettingsSection";
import { authState } from "../../stores/auth.store";
import { commands } from "../../ipc/commands";

const PrivacyTab: Component = () => {
  const [blockedUsers, setBlockedUsers] = createSignal<{ publicKey: string; displayName: string; blockedAt: number }[]>([]);

  onMount(() => {
    commands.getBlockedUsers().then(setBlockedUsers).catch((e) => {
      console.error("Failed to load blocked users:", e);
    });
  });

  async function handleUnblock(publicKey: string): Promise<void> {
    try {
      await commands.unblockUser(publicKey);
      setBlockedUsers((prev) => prev.filter((u) => u.publicKey !== publicKey));
    } catch (e) {
      console.error("Failed to unblock user:", e);
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
