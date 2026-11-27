import { type Component, For, Show, createResource, createSignal, onCleanup, onMount } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { commands, type LobbyRefusal, type LobbyVerdict, type LobbyView } from "../../ipc/commands";
import { subscribeGroupDmEvents } from "../../ipc/channels";
import { authState } from "../../stores/auth.store";
import { errorMessage } from "../../utils/error";

/** The Host's refusal, in words: the disabled button's title and accessible description. */
const REFUSAL: Record<LobbyRefusal, string> = {
  notOpen: "The lobby is not open",
  closed: "The lobby is closed",
  ownLobby: "You host this lobby",
  notHost: "Only the host can do this",
  alreadyIn: "You are already in the lobby",
  notIn: "You are not in the lobby",
  noMembers: "No one else has joined yet",
};

const refusal = (v: LobbyVerdict): string | null => (v.allowed ? null : REFUSAL[v.reason]);

/**
 * A group's game lobby (plan E7.2 G6): the game, its members, and Join,
 * Leave, Launch and Stop Hosting, each always rendered and disabled with
 * the Host's reason when its verdict refuses it. Once the host launches,
 * a member joins the game with one confirm, `GameLaunch{source: lobby}`
 * through the Host, whose review names the server; nothing launches by
 * itself. The host may name the server Launch uses (blank: the server
 * our game reports). With no lobby the group offers Host a game lobby (the Host picks
 * the game we are shown in). `lobby_get` on mount, then the `groupDm`
 * `lobby` event.
 */
const GameLobbyCard: Component<{ groupId: string; memberNames: Record<string, string> }> = (props) => {
  const [lobby, setLobby] = createSignal<LobbyView | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  // The server a launch names (host:port); blank lets the Host take our
  // game's own server (lobby.rs launch_server). The Host validates it.
  const [server, setServer] = createSignal("");
  const [gameName] = createResource(
    () => lobby()?.gameId,
    async (id) => (await commands.getGameName(id)) ?? `Game ${id}`,
  );
  const label = (key: string): string => props.memberNames[key] ?? (key === authState.publicKey ? "You" : "A member");
  const reasonId = (action: string): string => `lobby-${action}-reason-${props.groupId}`;

  let unlisten: UnlistenFn | undefined;
  onMount(async () => {
    unlisten = await subscribeGroupDmEvents((event) => {
      if ("lobby" in event && event.lobby.groupId === props.groupId) setLobby(event.lobby.lobby);
    });
    try {
      setLobby(await commands.lobbyGet(props.groupId));
    } catch (e) {
      setError(errorMessage(e));
    }
  });
  onCleanup(() => unlisten?.());

  /** Run a lobby command; its answer is the lobby as the Host now holds it. */
  async function act(run: () => Promise<LobbyView | null>): Promise<void> {
    setError(null);
    try {
      setLobby(await run());
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function joinGame(view: LobbyView, server: string): Promise<void> {
    setError(null);
    try {
      await commands.launchGame(view.gameId, server, "lobby");
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  const ActionButton: Component<{ action: string; text: string; verdict: LobbyVerdict; run: () => Promise<LobbyView | null> }> = (b) => (
    <>
      <button
        class="form-btn-secondary group-chat-action"
        aria-disabled={refusal(b.verdict) !== null ? "true" : undefined}
        aria-describedby={refusal(b.verdict) !== null ? reasonId(b.action) : undefined}
        title={refusal(b.verdict) ?? b.text}
        onClick={() => refusal(b.verdict) === null && void act(b.run)}
      >
        {b.text}
      </button>
      <span class="sr-only" id={reasonId(b.action)}>{refusal(b.verdict)}</span>
    </>
  );

  return (
    <section class="game-lobby" aria-label="Game lobby">
      <Show
        when={lobby()}
        fallback={
          <button class="form-btn-secondary group-chat-action" onClick={() => void act(() => commands.lobbyHost(props.groupId))}>
            Host a game lobby
          </button>
        }
      >
        {(view) => (
          <>
            <div class="game-lobby-title">
              <span class="game-lobby-game">{gameName() ?? ""}</span>
              <span class="game-lobby-host">hosted by {label(view().hostKey)}</span>
            </div>
            <ul class="game-lobby-members" aria-label="In the lobby">
              <For each={view().members}>{(key) => <li class="game-lobby-member">{label(key)}</li>}</For>
            </ul>
            <div class="game-lobby-actions">
              <ActionButton action="join" text="Join" verdict={view().join} run={() => commands.lobbyJoin(props.groupId, view().lobbyId)} />
              <ActionButton action="leave" text="Leave" verdict={view().leave} run={() => commands.lobbyLeave(props.groupId, view().lobbyId)} />
              <ActionButton
                action="launch"
                text="Launch"
                verdict={view().launch}
                run={() => commands.lobbyLaunch(props.groupId, view().lobbyId, server().trim() || null)}
              />
              <ActionButton action="close" text="Stop Hosting" verdict={view().close} run={() => commands.lobbyClose(props.groupId, view().lobbyId)} />
              <Show when={view().launch.allowed}>
                <input
                  class="form-input game-lobby-server"
                  aria-label="Server to launch on (blank: your game's server)"
                  placeholder="host:port"
                  value={server()}
                  onInput={(e) => setServer(e.currentTarget.value)}
                />
              </Show>
            </div>
            <Show when={view().state.kind === "launched" && view().hostKey !== authState.publicKey ? (view().state as { server: string }).server : null}>
              {(server) => (
                <div class="game-lobby-launched" role="status">
                  <span>The game is on at {server()}</span>
                  <button class="form-btn-primary group-chat-action" onClick={() => void joinGame(view(), server())}>
                    Join the game
                  </button>
                </div>
              )}
            </Show>
          </>
        )}
      </Show>
      <Show when={error()}>{(message) => <div class="form-error game-lobby-error" role="alert">{message()}</div>}</Show>
    </section>
  );
};

export default GameLobbyCard;
