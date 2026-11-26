// The Servers view (plan E7.2 G3): mirrors `rekindle_types::dto::servers`.
// The Host composes every category and row and states each row's verdicts;
// the window renders them and sends the row's own actions.
import { hostInvoke } from "../invoke";
import type { JoinSource, NotJoinableReason } from "./types";

/** Why a row's Add to Favorites or Remove is refused (`ServerRefusal`). */
export type ServerRefusal = "alreadyFavourite" | "noNumericGame" | "badAddress" | "notRemovable" | "refreshing";

/** A server as the last Refresh found it (`ServerInfo`, plan E7.2 G4): never queried, being
 *  queried, its answer, or why there was none (the Host's words). */
export type ServerInfo =
  | { kind: "notQueried" }
  | { kind: "querying" }
  | { kind: "answered"; name: string; map: string; players: number; maxPlayers: number; pingMs: number }
  | { kind: "unanswered"; reason: string };

/** One of a row's actions, as the Host allows it (`ServerVerdict`): the reason is present
 *  exactly when refused. */
export type ServerVerdict = { allowed: true } | { allowed: false; reason: ServerRefusal };

/** A row's Join: sent as `launch_game{gameId, server, source: launchSource}` when
 *  joinable; `joinableReason` is present exactly when it is not. */
type ServerJoin = {
  gameId: number | null;
  server: string | null;
  launchSource: Exclude<JoinSource, "installed">;
} & ({ joinable: true } | { joinable: false; joinableReason: NotJoinableReason });

export interface ServerRow {
  /** null: a community entry whose game id is not numeric. */
  gameId: number | null;
  gameName: string;
  iconKey: string | null;
  address: string;
  /** The favourite's or community's label; "" for recent and friends rows. */
  label: string;
  source: "favourite" | "friendPresence" | "invite" | "community" | "recent";
  /** Friends rows only. */
  friend: { publicKey: string; displayName: string } | null;
  /** Recent rows: unix seconds. */
  lastSeen: number | null;
  join: ServerJoin;
  /** What the last Refresh learned of the server (plan E7.2 G4); a server is queried only on Refresh. */
  info: ServerInfo;
  addFavourite: ServerVerdict;
  /** Favourites → FavouriteServerRemove, recent → RecentServerRemove. */
  remove: ServerVerdict;
}

export interface ServerCategory {
  kind: "favourites" | "recent" | "friends" | "community";
  /** Set only for kind "community". */
  community: { anchor: string; name: string } | null;
  /** A category with no rows is still sent. */
  rows: ServerRow[];
}

/** Ordered: favourites, recent, friends, then one per community. */
export interface ServersView {
  categories: ServerCategory[];
  /** Whether Refresh may run now (refused while one is running, with the Host's reason). */
  refresh: ServerVerdict;
}

export const serverCommands = {
  serversView: () => hostInvoke<ServersView>("servers_view"),
  recentServerRemove: (gameId: number, server: string) =>
    hostInvoke<ServersView>("recent_server_remove", { gameId, server }),
  /** Query every listed server once (the user's Refresh: a query shows our IP to the server). */
  serversRefresh: () => hostInvoke<{ started: boolean }>("servers_refresh"),
};
