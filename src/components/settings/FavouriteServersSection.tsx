import { type Component, createSignal, For, onMount, Show } from "solid-js";
import type { FavouriteServerView } from "../../ipc/commands";
import { commands } from "../../ipc/commands";
import type { GameTotalView } from "../../ipc/commands/dto";

/// Personal favourite game servers (plan E7.2): a game we have played, a
/// host:port and a label. The Host parses the address and owns the list;
/// each change answers with it as stored. A friend's chat window offers
/// these as game invites.
const FavouriteServersSection: Component = () => {
  const [favourites, setFavourites] = createSignal<FavouriteServerView[]>([]);
  const [played, setPlayed] = createSignal<GameTotalView[]>([]);
  const [game, setGame] = createSignal<number | null>(null);
  const [address, setAddress] = createSignal("");
  const [label, setLabel] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);

  onMount(() => {
    commands
      .getFavouriteServers()
      .then(setFavourites)
      .catch((e) => setError(String(e)));
    commands
      .getGameTotals()
      .then(setPlayed)
      .catch((e) => setError(String(e)));
  });

  async function change(run: () => Promise<FavouriteServerView[]>): Promise<boolean> {
    setError(null);
    try {
      setFavourites(await run());
      return true;
    } catch (e) {
      setError(`Favourite servers not changed: ${String(e)}`);
      return false;
    }
  }

  async function add(): Promise<void> {
    const id = game();
    if (id === null) return;
    if (await change(() => commands.addFavouriteServer(id, address().trim(), label().trim()))) {
      setAddress("");
      setLabel("");
    }
  }

  return (
    <>
      <div class="settings-section-title">Favourite Servers</div>
      <Show when={error()}>
        <div class="form-error" role="alert">
          {error()}
        </div>
      </Show>
      <Show
        when={favourites().length > 0}
        fallback={<div class="settings-hint">No favourite servers.</div>}
      >
        <For each={favourites()}>
          {(fav) => (
            <div class="blocked-user-item">
              <span class="buddy-name">
                {fav.label || fav.address} ({fav.gameName}, {fav.address})
              </span>
              <button
                type="button"
                class="form-btn-secondary"
                aria-label={`Remove ${fav.label || fav.address}`}
                onClick={() =>
                  void change(() => commands.removeFavouriteServer(fav.gameId, fav.address))
                }
              >
                Remove
              </button>
            </div>
          )}
        </For>
      </Show>
      <Show when={played().length > 0}>
        <div class="favourite-server-form">
          <select
            class="form-select"
            aria-label="Game"
            value={game() ?? ""}
            onChange={(e) => setGame(e.currentTarget.value ? Number(e.currentTarget.value) : null)}
          >
            <option value="">Choose a game…</option>
            <For each={played()}>{(g) => <option value={g.gameId}>{g.gameName}</option>}</For>
          </select>
          <input
            class="form-input"
            aria-label="Server address (host:port)"
            placeholder="host:port"
            value={address()}
            onInput={(e) => setAddress(e.currentTarget.value)}
          />
          <input
            class="form-input"
            aria-label="Label"
            placeholder="Label"
            maxLength={64}
            value={label()}
            onInput={(e) => setLabel(e.currentTarget.value)}
          />
          <button
            type="button"
            class="form-btn-secondary"
            disabled={game() === null || address().trim() === ""}
            onClick={() => void add()}
          >
            Add
          </button>
        </div>
      </Show>
      <div class="settings-hint">
        A favourite can be sent to a friend as a game invite while the game is shown to friends
        (Privacy).
      </div>
    </>
  );
};

export default FavouriteServersSection;
