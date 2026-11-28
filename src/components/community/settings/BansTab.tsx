import { type Component, createEffect, createSignal, For, on, Show } from "solid-js";
import { handleGetBanList, handleUnbanMember } from "../../../actions/community.actions";
import type { CommunityBanView } from "../../../ipc/commands/dto";
import { communityState } from "../../../stores/community.store";
import SkinGlyph from "../../common/SkinGlyph";

interface BansTabProps {
  communityId: string;
  canBan: boolean;
}

const BansTab: Component<BansTabProps> = (props) => {
  const [banList, setBanList] = createSignal<CommunityBanView[]>([]);
  // The Host holds the list: read on open and again on each BansChanged
  // (`bansRevision`, plan E3.2 S9), a ban or unban from anywhere included;
  // only the newest read lands.
  let readSeq = 0;
  createEffect(
    on(
      () =>
        [
          props.canBan,
          props.communityId,
          communityState.communities[props.communityId]?.bansRevision ?? 0,
        ] as const,
      ([canBan, communityId]) => {
        if (!canBan) return;
        const seq = ++readSeq;
        void handleGetBanList(communityId).then((list) => {
          if (seq === readSeq) setBanList(list);
        });
      }
    )
  );

  async function handleUnban(pseudonymKey: string): Promise<void> {
    await handleUnbanMember(props.communityId, pseudonymKey);
  }

  return (
    <div class="settings-section">
      <Show
        when={banList().length > 0}
        fallback={<div class="settings-hint">No banned members.</div>}
      >
        <For each={banList()}>
          {(banned) => (
            <div class="settings-list-item">
              <div class="settings-list-info">
                <span class="settings-list-name">
                  {banned.displayName || banned.pseudonymKey.slice(0, 16)}
                </span>
              </div>
              <button
                type="button"
                class="form-btn-secondary"
                onClick={() => handleUnban(banned.pseudonymKey)}
              >
                <SkinGlyph name="refresh" /> Unban
              </button>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
};

export default BansTab;
