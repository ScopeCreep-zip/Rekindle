// The Servers view (plan E7.2 G3): mirrors `rekindle_types::dto::servers`.
// The Host composes every category and row and states each row's verdicts;
// the window renders them and sends the row's own actions.
import { hostInvoke } from "../invoke";
import type { JoinSource, NotJoinableReason } from "./types";

/** Why a row's Add to Favorites or Remove is refused (`ServerRefusal`). */
export type ServerRefusal = "alreadyFavourite" | "noNumericGame" | "badAddress" | "notRemovable";

/** One of a row's actions, as the Host allows it (`ServerVerdict`). */
export interface ServerVerdict {
  allowed: boolean;
  reason: ServerRefusal | null;
}

/** A row's Join: sent as `launch_game{gameId, server, source: launchSource}` when joinable. */
export interface ServerJoin {
  gameId: number | null;
  server: string | null;
  launchSource: JoinSource;
  joinable: boolean;
  joinableReason: NotJoinableReason | null;
}

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
}

export const serverCommands = {
  serversView: () => hostInvoke<ServersView>("servers_view"),
  recentServerRemove: (gameId: number, server: string) =>
    hostInvoke<ServersView>("recent_server_remove", { gameId, server }),
};
