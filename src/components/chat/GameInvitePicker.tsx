import { type Component, For, Show, createSignal, onMount } from "solid-js";
import { commands } from "../../ipc/commands";
import type { FavouriteServerView } from "../../ipc/commands";

/** Invite this friend to one of our favourite servers (plan E7.2). Only
 *  the favourites the Host marks `invitable` are offered (not a hidden
 *  game, and current-game sharing on); the Host checks again on send. */
const GameInvitePicker: Component<{ peerId: string; peerName: string }> = (props) => {
  const [favourites, setFavourites] = createSignal<FavouriteServerView[]>([]);
  const [status, setStatus] = createSignal<string | null>(null);

  onMount(() => {
    commands
      .getFavouriteServers()
      .then((all) => setFavourites(all.filter((f) => f.invitable)))
      .catch((e) => setStatus(`Favourite servers not read: ${String(e)}`));
  });

  const send = async (fav: FavouriteServerView) => {
    try {
      await commands.sendGameInvite(props.peerId, fav.gameId, fav.address);
      setStatus(`Invited ${props.peerName} to ${fav.gameName} at ${fav.address}.`);
    } catch (e) {
      setStatus(`Invite not sent: ${String(e)}`);
    }
  };

  return (
    <div>
      <Show
        when={favourites().length > 0}
        fallback={
          <div class="settings-hint">
            No server to invite to: add a favourite server under Settings,
            and show friends the game you play (Privacy).
          </div>
        }
      >
        <ul class="game-invite-list" aria-label={`Invite ${props.peerName} to a server`}>
          <For each={favourites()}>
            {(fav) => (
              <li class="game-invite">
                <span class="game-invite-text">
                  {fav.label || fav.gameName} <span class="game-invite-server">{fav.address}</span>
                </span>
                <button type="button" class="form-btn-primary" onClick={() => void send(fav)}>
                  Invite
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <div class="settings-hint" role="status">{status()}</div>
    </div>
  );
};

export default GameInvitePicker;
