import { Component, For, Show, createMemo, createSignal } from "solid-js";
import { communityState } from "../../stores/community.store";
import { voiceState } from "../../stores/voice.store";
import { handlePlaySoundboard } from "../../handlers/community.handlers";
import {
  USE_SOUNDBOARD,
  calculateBasePermissions,
  hasPermission,
} from "../../utils/permissions";

/// Plan §Failure 6 — soundboard panel rendered alongside the voice
/// controls. Lists every community Expression with `kind === "soundboard"`
/// and dispatches `play_soundboard` on click. Gated by USE_SOUNDBOARD
/// (bit 39) so members without the permission don't see the panel.
const SoundboardPanel: Component = () => {
  const [collapsed, setCollapsed] = createSignal(true);

  const community = createMemo(() => {
    if (voiceState.activeCallType !== "community") return null;
    const id = communityState.activeCommunity;
    if (!id) return null;
    return communityState.communities[id] ?? null;
  });

  const sounds = createMemo(() => {
    const c = community();
    if (!c) return [];
    return c.expressions.filter((e) => e.kind === "soundboard");
  });

  const canUse = createMemo(() => {
    const c = community();
    if (!c) return false;
    const perms = calculateBasePermissions(c.myRoleIds, c.roles);
    return hasPermission(perms, USE_SOUNDBOARD);
  });

  function play(expressionId: string): void {
    const c = community();
    const channelId = voiceState.channelId;
    if (!c || !channelId) return;
    void handlePlaySoundboard(c.id, channelId, expressionId);
  }

  return (
    <Show when={voiceState.isConnected && canUse() && sounds().length > 0}>
      <div class="soundboard-panel">
        <button
          type="button"
          class="soundboard-panel-header"
          aria-expanded={!collapsed()}
          onClick={() => setCollapsed((v) => !v)}
        >
          <span>Soundboard</span>
          <span class="soundboard-panel-count">{sounds().length}</span>
        </button>
        <Show when={!collapsed()}>
          <div class="soundboard-panel-grid" role="list">
            <For each={sounds()}>
              {(s) => (
                <button
                  type="button"
                  role="listitem"
                  class="soundboard-panel-clip"
                  title={s.name}
                  onClick={() => play(s.id)}
                >
                  <span class="soundboard-panel-clip-name">{s.name}</span>
                </button>
              )}
            </For>
          </div>
        </Show>
      </div>
    </Show>
  );
};

export default SoundboardPanel;
