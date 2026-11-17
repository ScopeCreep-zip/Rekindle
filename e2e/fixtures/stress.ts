// Stress data for the layout suite (plan E7.12): the longest names and the
// largest lists a window has to lay out, as mock IPC handler source for
// `preloadMocks` (e2e/fixtures/mocks.ts). Every builder returns plain JSON,
// so it crosses the page boundary as a literal.

/** A display name of `n` characters with no spaces: the worst case for wrapping. */
export function longName(prefix: string, n = 64): string {
  return (prefix + "W".repeat(n)).slice(0, n);
}

/** A 64-hex identity key whose first character is `c` and tail is `i`. */
export function key(c: string, i = 0): string {
  const tail = i.toString(16).padStart(6, "0");
  return (c.repeat(58) + tail).slice(0, 64);
}

export const ME = key("a");
export const PEER = key("b");
export const COMMUNITY = `VLD0:${"g".repeat(43)}`;
export const GROUP = "0123456789abcdef0123456789abcdef";
export const CALL = "0192f0c4-7b3a-7c1e-9a55-1f0e2d3c4b5a";
export const CHANNEL = "ch0";

/** An unbroken URL longer than any window is wide. */
export const LONG_URL = `https://example.com/${"averyveryverylongpathsegment".repeat(12)}?q=${"x".repeat(80)}`;

const STATUSES = ["online", "away", "busy", "offline"] as const;

export interface FriendRow {
  publicKey: string;
  displayName: string;
  nickname: string | null;
  status: string;
  statusMessage: string | null;
  gameInfo: null;
  group: string | null;
  unreadCount: number;
  lastSeenAt: number | null;
  friendshipState: string;
  handshakeError: null;
  handshakeRetrying: boolean;
}

/** `n` accepted friends with 64-char names, status messages and unread badges. */
export function friends(n: number): FriendRow[] {
  return Array.from({ length: n }, (_, i) => ({
    publicKey: i === 0 ? PEER : key("c", i),
    displayName: longName(`Friend${i}_`),
    nickname: null,
    status: STATUSES[i % STATUSES.length],
    statusMessage: i % 3 === 0 ? longName(`Status${i}_`, 80) : null,
    gameInfo: null,
    group: i % 5 === 0 ? longName("Group_", 40) : null,
    unreadCount: i % 4 === 0 ? 120 + i : 0,
    lastSeenAt: null,
    friendshipState: "accepted",
    handshakeError: null,
    handshakeRetrying: false,
  }));
}

/** `n` community members with 64-char names spread over the presence states. */
export function members(n: number) {
  return Array.from({ length: n }, (_, i) => ({
    pseudonymKey: i === 0 ? ME : key("d", i),
    displayName: longName(`Member${i}_`),
    roleIds: [0],
    displayRole: "@everyone",
    status: STATUSES[i % STATUSES.length],
    timeoutUntil: null,
    lastActive: 1_700_000_000_000,
  }));
}

/** One community with a long name, many long-named channels and an @everyone that may post. */
export function communityDetails(channelCount = 30) {
  return [
    {
      id: COMMUNITY,
      name: longName("Community_"),
      description: longName("Description_", 200),
      channels: Array.from({ length: channelCount }, (_, i) => ({
        id: `ch${i}`,
        name: longName(`channel-${i}-`, 48),
        channelType: i % 6 === 5 ? "voice" : "text",
        topic: longName("Topic_", 120),
      })),
      roles: [
        { id: 0, name: "@everyone", color: 0, permissions: "65536", position: 0, hoist: false, mentionable: false },
      ],
      myRole: null,
      myRoleIds: [0],
      myPseudonymKey: ME,
      mekGeneration: 1,
      onboardingComplete: true,
      removed: null,
    },
  ];
}

export function communitiesList(n: number) {
  return Array.from({ length: n }, (_, i) => ({
    governanceKey: i === 0 ? COMMUNITY : `VLD0:${String(i).padStart(43, "h")}`,
    name: longName(`Community${i}_`),
    description: longName("Description_", 120),
    memberCount: 100,
    channelCount: 30,
    ourPseudonym: ME,
  }));
}

/** DM history: long words, a long unbroken URL and ordinary lines. */
export function dmHistory(n: number) {
  return Array.from({ length: n }, (_, i) => ({
    id: i + 1,
    messageId: `0192f0c4-7b3a-7c1e-9a55-${String(i).padStart(12, "0")}`,
    senderId: i % 2 ? ME : PEER,
    body: i === n - 1 ? LONG_URL : i % 3 === 0 ? longName("Unbroken_", 200) : `message ${i} with some ordinary words`,
    timestamp: 1_700_000_000_000 + i * 60_000,
    isOwn: i % 2 === 1,
    sendState: "delivered",
    historyGapThrough: null,
  }));
}

export function channelMessages(n: number) {
  return dmHistory(n).map(({ id, senderId, body, timestamp, isOwn }) => ({
    id,
    senderId,
    body,
    timestamp,
    isOwn,
  }));
}

export interface Stress {
  friends: number;
  members: number;
  messages: number;
  communities: number;
}

export const FULL: Stress = { friends: 100, members: 100, messages: 40, communities: 12 };
export const EMPTY: Stress = { friends: 0, members: 0, messages: 0, communities: 0 };

/**
 * The handler source for one stress level, showing a skin on `base`
 * (plan E7.7.2's `get_skin`). `get_*`/`list_*` commands not
 * named answer an empty list, the rest `null`, as a window with no data
 * would see them.
 */
export function stressHandler(s: Stress, base: "dark" | "light" = "dark"): string {
  const j = JSON.stringify;
  const call = {
    callId: CALL,
    peerKey: PEER,
    displayName: longName("Caller_"),
    kind: "audio",
    expiresAtMs: 4_102_444_800_000,
    connected: true,
  };
  return `
  switch (cmd) {
    case "subscribe_events":
      window.__eventChannelId = args.channel.id;
      return null;
    case "list_identities":
      return [{ publicKey: ${j(ME)}, displayName: ${j(longName("Me_"))}, createdAt: 1000 }];
    case "get_identity":
      return { publicKey: ${j(ME)}, displayName: ${j(longName("Me_"))} };
    case "get_preferences":
      return { notificationsEnabled: true, notificationSound: true, startMinimized: false, autoStart: false,
        gameDetectionEnabled: true, gameScanIntervalSecs: 30, readReceiptsEnabled: false };
    case "get_friends":
      return ${j(friends(s.friends))};
    case "get_communities":
      return ${j(communitiesList(s.communities))};
    case "get_community_details":
      return ${j(s.communities ? communityDetails() : [])};
    case "get_community_members":
      return ${j(members(s.members))};
    case "get_message_history":
      return ${j(dmHistory(s.messages))};
    case "get_channel_messages":
      return ${j(channelMessages(s.messages))};
    case "group_dm_list":
      return ${j(
        s.friends
          ? [{ groupId: GROUP, name: longName("Group_DM_"), epoch: 0, pending: false,
              members: friends(8).map((f) => ({ key: f.publicKey, label: f.displayName })) }]
          : [],
      )};
    case "group_dm_messages":
      return ${j(dmHistory(s.messages))};
    case "get_skin":
      return { id: "builtin:symbiosis", base: ${j(base)}, regions: [],
        tokensHref: ${j(`skin://localhost/${ME}/tokens.css`)} };
    case "get_active_call":
      return ${j(call)};
    default:
      return cmd.startsWith("get_") || cmd.startsWith("list_") ? [] : null;
  }
`;
}
