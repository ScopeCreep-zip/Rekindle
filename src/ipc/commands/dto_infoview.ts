// The Infoview's wire types (plan E7.15, IV4; `rekindle_types::dto::infoview`):
// what `get_infoview` answers and `infoview-event` pushes to the one window
// that asked. The Host owns the sections, their order and their actions.

import type { ActivityListItem, EventInfo, FriendInfo, NotJoinableReason } from "./types";

/// What a window's Infoview shows (`InfoviewSubject`).
type InfoviewSubject =
  | { type: "own" }
  | { type: "friend"; publicKey: string }
  | { type: "friendGroup"; groupId: number | null }
  | { type: "community"; community: string };

/// What a window asks its Infoview for: a subject, or `selection`, which the Host
/// resolves from the asking window's launcher's skin selection (plan E7.15 IV5;
/// a pack's Info popup shows what the buddy list has selected).
export type InfoviewRequest = InfoviewSubject | { type: "selection" };

/// Where a Join row's server came from, stated by the Host (`JoinSource`).
type JoinSource = "friendPresence" | "community";

/// What a row's link does (`InfoviewAction`): sent like any other action.
export type InfoviewAction =
  | { type: "openCommunity"; community: string }
  | {
      type: "joinServer";
      gameId: number | null;
      server: string | null;
      source: JoinSource;
      /// The Host's verdict (as `GameInfoState`): Join is always shown,
      /// disabled with `joinableReason` when not joinable.
      joinable: boolean;
      joinableReason: NotJoinableReason | null;
    }
  | { type: "openChat"; publicKey: string }
  | { type: "openProfile"; publicKey: string }
  | { type: "activityMore"; before: number };

/// A joined community on our own page (`InfoviewCommunityRow`).
interface InfoviewCommunityRow {
  anchor: string;
  name: string;
  unread: number;
  mentions: number;
  nextEvent: EventInfo | null;
  action: InfoviewAction;
}

/// A friend in game (`InfoviewPlayingRow`).
interface InfoviewPlayingRow {
  friend: FriendInfo;
  gameId: number;
  gameName: string;
  iconKey: string | null;
  elapsedSeconds: number;
  serverInfo: string | null;
  join: InfoviewAction;
}

/// A community's game server (`InfoviewServerRow`).
interface InfoviewServerRow {
  label: string;
  address: string;
  gameId: string;
  join: InfoviewAction;
}

/// A removal from a community as shown (`display::CommunityRemovalDisplay`).
interface InfoviewRemoval {
  banned: boolean;
  by: string | null;
  byName: string | null;
  reason: string | null;
}

/// A pinned channel message (`dto::community::PinnedMessageView`); `pinnedAt` in seconds.
interface InfoviewPin {
  messageId: string;
  channelId: string;
  pinnedBy: string;
  pinnedAt: number;
}

/// One section of a view (`InfoviewSection`).
export type InfoviewSection =
  | { type: "activity"; entries: ActivityListItem[]; more: InfoviewAction | null }
  | { type: "communities"; rows: InfoviewCommunityRow[] }
  | { type: "nowPlaying"; rows: InfoviewPlayingRow[] }
  | { type: "friendHeader"; friend: FriendInfo; blocked: boolean; actions: InfoviewAction[] }
  | { type: "bio"; text: string }
  | { type: "groupMembers"; label: string; online: FriendInfo[]; offline: FriendInfo[] }
  | {
      type: "communityHeader";
      anchor: string;
      name: string;
      description: string;
      memberCount: number;
      onlineCount: number;
      removed: InfoviewRemoval | null;
    }
  | { type: "pinned"; pins: InfoviewPin[] }
  | { type: "events"; events: EventInfo[] }
  | { type: "gameServers"; servers: InfoviewServerRow[] };

/// One window's Infoview (`InfoviewView`).
export interface InfoviewView {
  /** Always the resolved subject, never `selection`. */
  subject: InfoviewSubject;
  sections: InfoviewSection[];
}
