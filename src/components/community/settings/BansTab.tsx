import { Component, For, Show, createSignal, createEffect } from "solid-js";
import { handleGetBanList, handleUnbanMember } from "../../../handlers/community.handlers";
import { ICON_REFRESH } from "../../../icons";

interface BansTabProps {
  communityId: string;
  canBan: boolean;
}

const BansTab: Component<BansTabProps> = (props) => {
  const [banList, setBanList] = createSignal<{ pseudonymKey: string; displayName: string; bannedAt: number }[]>([]);
  const [loaded, setLoaded] = createSignal(false);

  createEffect(() => {
    if (props.canBan && !loaded()) {
      setLoaded(true);
      handleGetBanList(props.communityId).then(setBanList);
    }
  });

  async function handleUnban(pseudonymKey: string): Promise<void> {
    await handleUnbanMember(props.communityId, pseudonymKey);
    setBanList((prev) => prev.filter((b) => b.pseudonymKey !== pseudonymKey));
  }

  return (
    <div class="settings-section">
      <Show when={banList().length > 0} fallback={
        <div class="settings-hint">No banned members.</div>
      }>
        <For each={banList()}>
          {(banned) => (
            <div class="settings-list-item">
              <div class="settings-list-info">
                <span class="settings-list-name">{banned.displayName || banned.pseudonymKey.slice(0, 16)}</span>
                <span class="settings-list-date">
                  Banned {new Date(banned.bannedAt * 1000).toLocaleDateString()}
                </span>
              </div>
              <button
                class="form-btn-secondary"
                onClick={() => handleUnban(banned.pseudonymKey)}
              >
                <span class="nf-icon">{ICON_REFRESH}</span> Unban
              </button>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
};

export default BansTab;
