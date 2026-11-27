import { type Component, For, Show, createMemo, createSignal, onMount } from "solid-js";
import { commands } from "../../ipc/commands";
import type { HiddenGameView } from "../../ipc/commands";
import type { GameTotalView } from "../../ipc/commands/dto";

/// The hide list (plan E7.2): games never shown to anyone, picked from the
/// games we have played. The Host owns the list; each change answers with
/// it as stored.
const HiddenGamesSection: Component = () => {
  const [hidden, setHidden] = createSignal<HiddenGameView[]>([]);
  const [played, setPlayed] = createSignal<GameTotalView[]>([]);
  const [pick, setPick] = createSignal<number | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  onMount(() => {
    commands
      .getHiddenGames()
      .then(setHidden)
      .catch((e) => setError(String(e)));
    commands
      .getGameTotals()
      .then(setPlayed)
      .catch((e) => setError(String(e)));
  });

  const hideable = createMemo(() =>
    played().filter((game) => !hidden().some((h) => h.gameId === game.gameId))
  );

  async function change(run: () => Promise<HiddenGameView[]>): Promise<void> {
    setError(null);
    try {
      setHidden(await run());
      setPick(null);
    } catch (e) {
      setError(`Hidden games not changed: ${String(e)}`);
    }
  }

  return (
    <>
      <div class="settings-section-title">Hidden Games</div>
      <Show when={error()}>
        <div class="form-error">{error()}</div>
      </Show>
      <Show when={hidden().length > 0} fallback={<div class="settings-hint">No hidden games.</div>}>
        <For each={hidden()}>
          {(game) => (
            <div class="blocked-user-item">
              <span class="buddy-name">{game.gameName}</span>
              <button
                type="button"
                class="form-btn-secondary"
                onClick={() => void change(() => commands.unhideGame(game.gameId))}
              >
                Unhide
              </button>
            </div>
          )}
        </For>
      </Show>
      <Show when={hideable().length > 0}>
        <div class="form-field-row">
          <select
            class="form-select"
            aria-label="Game to hide"
            value={pick() ?? ""}
            onChange={(e) => setPick(e.currentTarget.value ? Number(e.currentTarget.value) : null)}
          >
            <option value="">Choose a game…</option>
            <For each={hideable()}>
              {(game) => <option value={game.gameId}>{game.gameName}</option>}
            </For>
          </select>
          <button
            type="button"
            class="form-btn-secondary"
            disabled={pick() === null}
            onClick={() => {
              const id = pick();
              if (id !== null) void change(() => commands.hideGame(id));
            }}
          >
            Hide
          </button>
        </div>
      </Show>
      <div class="settings-hint">
        A hidden game is never shown to friends, nor in the game time they see. It still counts
        toward your own game time.
      </div>
    </>
  );
};

export default HiddenGamesSection;
