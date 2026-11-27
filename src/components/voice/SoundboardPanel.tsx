import { type Component, For, Show, createMemo, createSignal } from "solid-js";
import { communityState } from "../../stores/community.store";
import { voiceState } from "../../stores/voice.store";
import { handlePlaySoundboard } from "../../actions/community.actions";
import { channelDeniedFor } from "../../utils/denied";

/// Plan §Failure 6 — soundboard panel rendered alongside the voice
/// controls. Lists every community Expression with `kind === "soundboard"`
/// and dispatches `play_soundboard` on click. Never hidden for a missing
/// permission: each sound is drawn disabled with the Host's reason for the
/// joined voice channel (CAPR `useSoundboard`, judged with its overwrites).
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

  // The Host's verdict for the voice channel we are in, or null when we may.
  const useDenied = createMemo(() => channelDeniedFor(community(), voiceState.channelId, "useSoundboard"));

  function play(expressionId: string): void {
    const c = community();
    const channelId = voiceState.channelId;
    if (!c || !channelId) return;
    void handlePlaySoundboard(c.id, channelId, expressionId);
  }

  return (
    <Show when={voiceState.isConnected && sounds().length > 0}>
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
          <ul class="soundboard-panel-grid">
            <For each={sounds()}>
              {(s) => (
                <li class="soundboard-panel-item">
                  <button
                    type="button"
                    class="soundboard-panel-clip"
                    title={useDenied() ?? s.name}
                    aria-description={useDenied() ?? undefined}
                    disabled={useDenied() !== null}
                    onClick={() => play(s.id)}
                  >
                    <span class="soundboard-panel-clip-name">{s.name}</span>
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </div>
    </Show>
  );
};

export default SoundboardPanel;
