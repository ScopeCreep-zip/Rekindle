import { Component, For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { subscribeChatEvents } from "../../ipc/channels";
import { commands } from "../../ipc/commands";

interface Invite {
  gameId: number;
  server: string;
  gameName: string | null;
}

/** A friend's game invites in their chat window (plan E7.2). The Host sends
 *  only an accepted friend's, with the server already parsed; "Join" hands
 *  it to the game (`LaunchGameToServer`, the same act as "Join Game" from a
 *  friend's presence). */
const GameInviteList: Component<{ peerId: string; peerName: string }> = (props) => {
  const [invites, setInvites] = createSignal<Invite[]>([]);
  let unlisten: (() => void) | undefined;

  onMount(async () => {
    unlisten = await subscribeChatEvents((event) => {
      if (!("friend" in event)) return;
      const friend = event.friend;
      if (!("gameInvite" in friend) || friend.gameInvite.peerKey !== props.peerId) return;
      const { gameId, server } = friend.gameInvite;
      void commands.getGameName(gameId).then((gameName) => {
        // The newest invite for a game replaces an older one.
        setInvites((list) => [...list.filter((i) => i.gameId !== gameId), { gameId, server, gameName }]);
      });
    });
  });
  onCleanup(() => unlisten?.());

  const dismiss = (invite: Invite) => setInvites((list) => list.filter((i) => i !== invite));
  const join = async (invite: Invite) => {
    await commands.launchGameToServer(invite.gameId, invite.server);
    dismiss(invite);
  };

  return (
    <Show when={invites().length > 0}>
      <ul class="game-invite-list" aria-label={`Game invites from ${props.peerName}`}>
        <For each={invites()}>
          {(invite) => (
            <li class="game-invite">
              <span class="game-invite-text">
                {props.peerName} invites you to {invite.gameName ?? `game ${invite.gameId}`} at{" "}
                <span class="game-invite-server">{invite.server}</span>
              </span>
              <button type="button" class="form-btn-primary" onClick={() => void join(invite)}>
                Join
              </button>
              <button type="button" class="form-btn-secondary" onClick={() => dismiss(invite)}>
                Dismiss
              </button>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
};

export default GameInviteList;
