import { type Component, For, Show, createMemo, createSignal, createEffect } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";
import type { GameServer } from "../../stores/community.store";
import { commands } from "../../ipc/commands";

interface GameServerListProps {
  servers: GameServer[];
  communityId: string;
  /** Why we may not manage the server list, in the Host's words, or null (never-hide). */
  manageDenied: string | null;
  onRemove: (communityId: string, serverId: string) => void;
  onAdd: (communityId: string, gameId: string, label: string, address: string) => void;
}

const GameServerList: Component<GameServerListProps> = (props) => {
  const [addGameId, setAddGameId] = createSignal("");
  const [addLabel, setAddLabel] = createSignal("");
  const [addAddress, setAddAddress] = createSignal("");
  const [nameCache, setNameCache] = createSignal<Map<string, string>>(new Map());

  // Resolve numeric game IDs to human-readable names
  createEffect(() => {
    const servers = props.servers;
    const cache = nameCache();
    const numericIds = new Set<string>();
    for (const s of servers) {
      if (s.gameId.match(/^\d+$/) && !cache.has(s.gameId)) {
        numericIds.add(s.gameId);
      }
    }
    for (const id of numericIds) {
      commands.getGameName(parseInt(id, 10)).then((name) => {
        if (name) {
          setNameCache((prev) => {
            const next = new Map(prev);
            next.set(id, name);
            return next;
          });
        }
      });
    }
  });

  function resolveGameName(gameId: string): string {
    const cached = nameCache().get(gameId);
    if (cached) return cached;
    return gameId.match(/^\d+$/) ? `Game #${gameId}` : gameId;
  }

  const grouped = createMemo(() => {
    const groups: Record<string, GameServer[]> = {};
    for (const server of props.servers) {
      if (!groups[server.gameId]) groups[server.gameId] = [];
      groups[server.gameId].push(server);
    }
    return Object.entries(groups);
  });

  function handleAdd(): void {
    const gameId = addGameId().trim();
    const label = addLabel().trim();
    const address = addAddress().trim();
    if (!gameId || !label || !address) return;
    // Basic address validation: must contain : with content on both sides
    const colonIdx = address.indexOf(":");
    if (colonIdx < 1 || colonIdx >= address.length - 1) return;
    props.onAdd(props.communityId, gameId, label, address);
    setAddGameId("");
    setAddLabel("");
    setAddAddress("");
  }

  function handleJoin(gameId: string, address: string): void {
    const id = parseInt(gameId, 10);
    if (!Number.isNaN(id)) {
      void commands.launchGame(id, address, "community");
    }
  }

  return (
    <div class="game-server-list">
      <Show when={props.servers.length === 0}>
        <div class="pin-panel-empty">No game servers added</div>
      </Show>
      <For each={grouped()}>
        {([gameId, servers]) => (
          <div class="game-server-group">
            <div class="game-server-group-header">
              <SkinGlyph name="gamepad" />
              {resolveGameName(gameId)}
            </div>
            <For each={servers}>
              {(server) => (
                <div class="game-server-row">
                  <SkinGlyph name="server" class="game-server-icon" />
                  <div class="game-server-info">
                    <div class="game-server-label">{server.label}</div>
                    <div class="game-server-address">{server.address}</div>
                  </div>
                  <button
                    type="button"
                    class="game-server-join-btn"
                    onClick={() => handleJoin(server.gameId, server.address)}
                    title="Connect to server"
                  >
                    Join
                  </button>
                  <button type="button"
                    class="game-server-remove-btn"
                    onClick={() => props.onRemove(props.communityId, server.id)}
                    disabled={props.manageDenied !== null}
                    title={props.manageDenied ?? "Remove server"}
                    aria-label="Remove server"
                    aria-description={props.manageDenied ?? undefined}
                  >
                    <SkinGlyph name="delete" />
                  </button>
                </div>
              )}
            </For>
          </div>
        )}
      </For>
      <fieldset class="game-server-add-form" disabled={props.manageDenied !== null} title={props.manageDenied ?? undefined} aria-description={props.manageDenied ?? undefined}>
          <input
            class="form-input"
            placeholder="Game ID"
            value={addGameId()}
            onInput={(e) => setAddGameId(e.currentTarget.value)}
          />
          <input
            class="form-input"
            placeholder="Label"
            value={addLabel()}
            onInput={(e) => setAddLabel(e.currentTarget.value)}
          />
          <input
            class="form-input"
            placeholder="ip:port"
            value={addAddress()}
            onInput={(e) => setAddAddress(e.currentTarget.value)}
            onKeyDown={(e) => { if (e.key === "Enter") handleAdd(); }}
          />
          <button type="button" class="form-btn-primary" onClick={handleAdd}>Add</button>
      </fieldset>
    </div>
  );
};

export default GameServerList;
