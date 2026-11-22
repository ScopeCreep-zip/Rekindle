import { Component, For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { handleLaunchGame } from "../../actions/game.actions";
import { sendSkinSelection } from "../../actions/skin.actions";
import { subscribeServersView } from "../../ipc/channels";
import { commands } from "../../ipc/commands";
import { serverCommands, type ServerCategory, type ServerInfo, type ServerRefusal, type ServerRow, type ServerVerdict, type ServersView as View } from "../../ipc/commands/dto_servers";
import { isCommandError } from "../../ipc/commands/dto";
import { addToast } from "../../stores/toast.store";
import { errorMessage } from "../../utils/error";
import { formatRelativeTime } from "../../utils/formatting";
import { notJoinableText } from "../../utils/joinable_text";

// The Servers view (Xfire's Servers tab, plan E7.2 G3): the categories and
// rows the Host composes (`servers_view`, then `servers-event`), each row
// with the Host's verdicts on Join, Add to Favorites and Remove; every
// button is listed, disabled with the Host's reason when not allowed. A row
// press makes a server with a known game the window's selection
// (`setSkinSelection` Server), which the pack's Add Server and Join act on.

const REFUSAL_TEXT: Record<ServerRefusal, string> = {
  alreadyFavourite: "already a favorite",
  noNumericGame: "its game is not known",
  badAddress: "not a valid server address",
  notRemovable: "can't be removed from this list",
  refreshing: "Refreshing...",
};

function categoryTitle(c: ServerCategory): string {
  switch (c.kind) {
    case "favourites":
      return "Favorites";
    case "recent":
      return "Recent";
    case "friends":
      return "Friends' servers";
    case "community":
      return c.community?.name ?? "Community";
  }
}

/** What the last Refresh learned, as one line (plan E7.2 G4): the server's name, map,
 *  players and ping; "Querying..." while it runs; the Host's reason when it had no answer. */
function infoText(info: ServerInfo): string {
  switch (info.kind) {
    case "notQueried":
      return "";
    case "querying":
      return "Querying...";
    case "answered":
      return `${info.name} Â· ${info.map} Â· ${info.players}/${info.maxPlayers} players Â· ${info.pingMs} ms`;
    case "unanswered":
      return info.reason;
  }
}

const rowKey = (r: ServerRow): string => `${r.source}|${r.gameId ?? ""}|${r.address}|${r.friend?.publicKey ?? ""}`;

/** One of a row's buttons: always listed, the Host's verdict enabling it. */
const RowButton: Component<{ label: string; refused: string | null; onPress: () => void }> = (props) => (
  <button
    type="button"
    class="servers-view-action"
    aria-disabled={props.refused ? "true" : undefined}
    title={props.refused ?? undefined}
    onClick={(e) => {
      e.stopPropagation();
      if (!props.refused) props.onPress();
    }}
  >
    {props.label}
  </button>
);

const verdictText = (v: ServerVerdict): string | null => (v.allowed ? null : REFUSAL_TEXT[v.reason]);

const ServersView: Component = () => {
  const [view, setView] = createSignal<View | null>(null);
  const [selected, setSelected] = createSignal<string | null>(null);
  let unlisten: (() => void) | undefined;

  onMount(() => {
    void subscribeServersView(setView).then((u) => (unlisten = u));
    serverCommands.serversView().then(setView).catch((e) => addToast(`Servers not read: ${errorMessage(e)}`, "error"));
  });
  onCleanup(() => unlisten?.());

  // A selected server the Host no longer lists is no longer the selection.
  createEffect(() => {
    const k = selected();
    const v = view();
    if (k !== null && v && !v.categories.some((c) => c.rows.some((r) => rowKey(r) === k))) {
      setSelected(null);
      void sendSkinSelection(null);
    }
  });

  const select = (r: ServerRow): void => {
    setSelected(rowKey(r));
    // A server whose game is not known (a community's non-numeric game id) selects nothing.
    void sendSkinSelection(r.gameId === null ? null : { kind: "server", arg: { gameId: r.gameId, address: r.address } });
  };

  const run = async (what: string, action: () => Promise<unknown>): Promise<void> => {
    try {
      const answer = await action();
      if (answer && typeof answer === "object" && "categories" in answer) setView(answer as View);
    } catch (e) {
      // A review the user cancelled (Refresh and Add are reviewed) is no failure.
      if (!(isCommandError(e) && e.type === "cancelled")) addToast(`${what}: ${errorMessage(e)}`, "error");
    }
  };

  const join = (r: ServerRow): void => {
    const j = r.join;
    if (j.joinable && j.gameId !== null) void handleLaunchGame(j.gameId, j.server, j.launchSource);
  };
  const addFavourite = (r: ServerRow): void => {
    if (r.gameId === null) return;
    const gameId = r.gameId;
    void run("Not added", () => commands.addFavouriteServer(gameId, r.address, r.label || r.gameName));
  };
  const remove = (r: ServerRow): void => {
    if (r.gameId === null) return;
    const gameId = r.gameId;
    void run(
      "Not removed",
      r.source === "recent"
        ? () => serverCommands.recentServerRemove(gameId, r.address)
        : () => commands.removeFavouriteServer(gameId, r.address),
    );
  };

  const meta = (r: ServerRow): string =>
    [r.friend?.displayName, r.address, r.lastSeen !== null ? formatRelativeTime(r.lastSeen * 1000) : null]
      .filter((s): s is string => !!s)
      .join(" ÃÂ· ");

  // Refresh queries every listed server once, only when the user asks, through
  // the Host's review (a query shows our IP to each server); the Host says
  // when it may run.
  const refreshRefused = (): string | null => {
    const v = view()?.refresh;
    return v ? verdictText(v) : "Servers not read yet";
  };
  const refresh = (): void => {
    if (refreshRefused() === null) void run("Not refreshed", () => serverCommands.serversRefresh());
  };

  return (
    <div class="servers-view" role="listbox" aria-label="Servers">
      <div class="servers-view-toolbar">
        <RowButton label="Refresh" refused={refreshRefused()} onPress={refresh} />
      </div>
      <For each={view()?.categories ?? []}>
        {(category) => (
          <div class="servers-view-section" role="group" aria-label={categoryTitle(category)}>
            <div class="servers-view-title">{categoryTitle(category)}</div>
            <Show when={category.rows.length > 0} fallback={<div class="servers-view-empty">None</div>}>
              <For each={category.rows}>
                {(r) => (
                  <div
                    class="servers-view-row"
                    role="option"
                    tabindex="0"
                    aria-selected={selected() === rowKey(r)}
                    onClick={() => select(r)}
                    onKeyDown={(e) => {
                      if (e.target === e.currentTarget && (e.key === " " || e.key === "Enter")) {
                        e.preventDefault();
                        select(r);
                      }
                    }}
                  >
                    <span class="servers-view-name">{r.label || r.gameName}</span>
                    <span class="servers-view-address">{meta(r)}</span>
                    <Show when={infoText(r.info)}>
                      {(text) => (
                        <span class="servers-view-info" data-info={r.info.kind}>
                          {text()}
                        </span>
                      )}
                    </Show>
                    <span class="servers-view-actions">
                      <RowButton
                        label="Join"
                        refused={r.join.joinable ? null : notJoinableText(r.join)}
                        onPress={() => join(r)}
                      />
                      <RowButton label="Add to Favorites" refused={verdictText(r.addFavourite)} onPress={() => addFavourite(r)} />
                      <RowButton label="Remove" refused={verdictText(r.remove)} onPress={() => remove(r)} />
                    </span>
                  </div>
                )}
              </For>
            </Show>
          </div>
        )}
      </For>
    </div>
  );
};

export default ServersView;
