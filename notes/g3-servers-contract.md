# G3 Servers view: contract (branch wt/g3srv, worktree /Users/kali/Rekindle-wt-g3srv, base 732ea690f)
(wt/g3 already exists from an older E2.6 slice, so this slice lives on wt/g3srv.)

## Requests (all through HostApi::call -> intent gate)
| Request (IpcRequest, serde `{"type":..}` as every IpcRequest) | Args | Class | Answer |
|---|---|---|---|
| `ServersView` | none | Query | `ServersView` |
| `RecentServerRemove` | `gameId: number, server: string` (parsed ServerAddress) | Routine (local) | `ServersView` |
| existing `FavouriteServerAdd {gameId, server, label}` / `FavouriteServerRemove {gameId, server}` | unchanged | Routine | unchanged (`FavouriteServerView[]`) |
| existing `GameLaunch {gameId, server, source}` | the row's `join` fields | per source (peer = TrustChanging) | unchanged; a launch with a server now also records a recent row |
Tauri commands: `servers_view()` -> ServersView, `recent_server_remove(gameId, server)` -> ServersView (ACL: buddy list window).

## DTOs (`rekindle-types/src/dto/servers.rs`, camelCase)
```ts
type ServersView = { categories: ServerCategory[] };            // order: favourites, recent, friends, then one per community
type ServerCategory = {
  kind: "favourites" | "recent" | "friends" | "community";
  community: { anchor: string; name: string } | null;            // set only for kind "community"
  rows: ServerRow[];                                              // a category with no rows is still sent
};
type ServerRow = {
  gameId: number | null;            // null: a community entry whose game id is not numeric
  gameName: string;                 // from the game database; "Game <id>" / the community's text when unknown
  iconKey: string | null;
  address: string;                  // host:port as stored/shared
  label: string;                    // favourite/community label; "" for recent/friends
  source: "favourite" | "friendPresence" | "invite" | "community" | "recent";
  friend: { publicKey: string; displayName: string } | null;     // friends rows only
  lastSeen: number | null;          // recent rows: unix seconds
  join: ServerJoin;                 // always present
  addFavourite: ServerVerdict;      // always present
  remove: ServerVerdict;            // always present (favourites -> FavouriteServerRemove, recent -> RecentServerRemove)
};
type ServerJoin = {                 // send as GameLaunch{gameId, server, source: launchSource} when joinable
  gameId: number | null; server: string | null;
  launchSource: "favourite" | "recent" | "friendPresence" | "invite" | "community";   // rekindle_types::game_server::JoinSource
} & ({ joinable: true } | { joinable: false; joinableReason: "noServer" | "cannotLaunch" });   // Rust: #[serde(flatten)] Joinability
type ServerVerdict = { allowed: true } | { allowed: false; reason: ServerRefusal };   // Rust: enum ServerVerdict { Allowed, Refused{reason} }, serde try_from/into a {allowed, reason?} struct; impossible pairs fail to decode
type ServerRefusal = "alreadyFavourite" | "noNumericGame" | "badAddress" | "notRemovable";
```
`launchSource` for recent rows is a new `JoinSource::Recent` (Routine: a server we ourselves visited, threat model I10 unchanged as our IP already went there). Favourite -> `favourite`, friend presence -> `friendPresence`, invite -> `invite`, community -> `community`.

## Event (refresh)
`SubscriptionEvent::System(SystemEvent::ServersViewChanged { view: ServersView })`, logical channel `"servers-event"`,
pushed by the Host's servers worker at most once per 250 ms after: FavouriteServerAdded, FavouriteServerRemove, RecentServerRemove,
a recent row written (own presence with a server, GameLaunch with a server), a friend's presence/friendship/GameInvite, a membership
change, a community game-server list change. Scope: Device. **Audience (desktop `event_dispatch::audience`)**: the buddy list
(the Servers tab, `EngineView::Servers`) plus every skin-tree window that can show the view (the same kinds Infoview/transfers use: buddy list + Chat).
TUI/CLI read `ServersView` and fold the same event.

## Skins
- `EngineView::Servers` (exists, trigger name `servers`): the buddy list's Servers tab shows this view.
- Selection: `setSkinSelection({kind:"server", arg:{gameId, address}})` for a row with a numeric gameId; a `gameId:null` row selects nothing.
- Pack actions: `AddServer` -> FavouriteServerAdd (capability `addserver` = Server selected and not a favourite, already in Host);
  `Join` on a server selection -> the row's ServerJoin (capability `join` = selected server's game launchable).
