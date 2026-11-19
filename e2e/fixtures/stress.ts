// Stress data for the layout suite (plan E7.12): the longest names and the
// largest lists a window has to lay out, as mock IPC handler source for
// `preloadMocks` (e2e/fixtures/mocks.ts). Every builder returns plain JSON,
// so it crosses the page boundary as a literal.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

/** The Host's role-tag colours for the fixture role colours, written from
 *  `rekindle_skin::role_tag` by `cargo xtask gen-skin-fixtures` (plan E7.3). */
const ROLE_TAGS: Record<string, { dark: number; light: number }> = JSON.parse(
  readFileSync(fileURLToPath(new URL("./generated/role-tags.json", import.meta.url)), "utf8"),
);

/** A role's `color` and the `tagColours` the Host sends with it. */
function roleColour(color: number) {
  // No colour draws the default tag, which reads no tag colours.
  if (color === 0) return { color, tagColours: { dark: 0, light: 0 } };
  const tagColours = ROLE_TAGS[color.toString(16).padStart(6, "0")];
  if (!tagColours) throw new Error(`role colour ${color.toString(16)} is not in xtask gen_skin_fixtures::ROLE_COLOURS`);
  return { color, tagColours };
}

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
  groupId: number | null;
  group: string | null;
  groupLabel: string;
  unreadCount: number;
  lastSeenAt: number | null;
  friendshipState: string;
  handshakeError: null;
  handshakeRetrying: boolean;
  identityPinMismatch: boolean;
}

/** `n` accepted friends with 64-char names, status messages and unread badges. */
export function friends(n: number): FriendRow[] {
  return Array.from({ length: n }, (_, i) => {
    const group = i % 5 === 0 ? longName("Group_", 40) : null;
    return {
      publicKey: i === 0 ? PEER : key("c", i),
      displayName: longName(`Friend${i}_`),
      nickname: null,
      status: STATUSES[i % STATUSES.length],
      statusMessage: i % 3 === 0 ? longName(`Status${i}_`, 80) : null,
      gameInfo: null,
      groupId: group === null ? null : 1,
      group,
      // The Host's label: the group, or its default group's name
      // (`rekindle_types::dto::friends::group_label`).
      groupLabel: group ?? "Friends",
      unreadCount: i % 4 === 0 ? 120 + i : 0,
      lastSeenAt: null,
      friendshipState: "accepted",
      handshakeError: null,
      handshakeRetrying: false,
      identityPinMismatch: false,
    };
  });
}

const HOISTED_ROLE = longName("Moderators_", 32);

/** `n` community members with 64-char names spread over the presence states;
 *  every fifth holds the hoisted long-named role, so the member list draws a
 *  hoisted group, "Online" and "Offline" (an offline role holder sits in "Offline").
 *  As the Host sends them: each with its `group`, ordered group by group. */
export function members(n: number) {
  const rank = { role: 0, online: 1, offline: 2 } as const;
  return Array.from({ length: n }, (_, i) => {
    const status = STATUSES[i % STATUSES.length];
    const present = ["online", "away", "busy"].includes(status);
    const hoisted = i % 5 === 0;
    const group = !present
      ? { kind: "offline" as const }
      : hoisted
        ? { kind: "role" as const, id: "01010101010101010101010101010101", name: HOISTED_ROLE }
        : { kind: "online" as const };
    return {
      pseudonymKey: i === 0 ? ME : key("d", i),
      displayName: longName(`Member${i}_`),
      roleIds: hoisted ? ["00000000000000000000000000000000", "01010101010101010101010101010101"] : ["00000000000000000000000000000000"],
      displayRole: hoisted ? HOISTED_ROLE : "@everyone",
      status,
      timeoutUntil: null,
      lastActive: 1_700_000_000_000,
      group,
    };
  }).sort((a, b) => rank[a.group.kind] - rank[b.group.kind]);
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
        { id: "00000000000000000000000000000000", name: "@everyone", ...roleColour(0), permissions: "65536", position: 0, hoist: false, mentionable: false },
        { id: "01010101010101010101010101010101", name: HOISTED_ROLE, ...roleColour(0x3498db), permissions: "65536", position: 1, hoist: true, mentionable: false },
      ],
      myRole: null,
      myRoleIds: ["00000000000000000000000000000000"], myPermissions: "65536", myChannelPermissions: {},
      myPseudonymKey: ME,
      mekGeneration: 1,
      onboardingComplete: true,
      removed: null,
    },
  ];
}

export function communitiesList(n: number) {
  return Array.from({ length: n }, (_, i) => ({
    anchor: i === 0 ? COMMUNITY : `VLD0:${String(i).padStart(43, "h")}`,
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

/** Channel messages: ours and a community member's (`members()` key 1), so
 *  the sender resolves to a member name as it does live. */
export function channelMessages(n: number) {
  return dmHistory(n).map(({ id, body, timestamp, isOwn }) => ({
    id,
    senderId: isOwn ? ME : key("d", 1),
    body,
    timestamp,
    isOwn,
  }));
}

/** A 33×33 module grid with the three finder patterns, as the Host's
 *  `qrRows` (`1` dark, no quiet zone): the shape of a real code. */
export function qrRows(n = 33): string[] {
  const finder = (x: number, y: number) => {
    const d = Math.max(Math.abs(x - 3), Math.abs(y - 3));
    return d !== 2 && d <= 3;
  };
  return Array.from({ length: n }, (_, y) =>
    Array.from({ length: n }, (_, x) => {
      if (x < 7 && y < 7) return finder(x, y) ? "1" : "0";
      if (x >= n - 7 && y < 7) return finder(x - (n - 7), y) ? "1" : "0";
      if (x < 7 && y >= n - 7) return finder(x, y - (n - 7)) ? "1" : "0";
      return (x * 7 + y * 13) % 3 === 0 ? "1" : "0";
    }).join(""),
  );
}

/** Our safety number with `peer` as `IdentitySafetyNumber` answers it
 *  (`rekindle_types::dto::friends::SafetyNumberView`): 60 digits, the
 *  scannable form in base64, its QR rows, never verified. */
export function safetyNumber(peer: string) {
  return {
    peerKey: peer,
    digits: Array.from({ length: 60 }, (_, i) => String((i * 7) % 10)).join(""),
    scannable: "CAESIgogq83vASNFZ4mrze8BI0VniavN7wEjRWeJq83vASNFZ4kaIgog",
    qrRows: qrRows(),
    verified: false,
    verifiedAt: null,
    changed: false,
  };
}

/** A set welcome screen (`WelcomeScreenDto`): long text, featured channels. */
export function welcomeScreen() {
  return {
    description: longName("Welcome_", 300),
    channels: Array.from({ length: 5 }, (_, i) => ({
      channelId: `ch${i}`,
      description: longName(`Featured${i}_`, 120),
      emoji: null,
    })),
  };
}

export interface Stress {
  friends: number;
  members: number;
  messages: number;
  communities: number;
}

/** Settings → Appearance's library: the built-ins, then (with data) a
 *  dozen long-named library skins, one of them on trial. */
/** Credits at their bounds (plan E7.7.3d): long names, a long unbroken
 *  URL and a multi-line licence. */
function skinCredits(i: number) {
  return {
    authors: [longName(`Author_${i}_`), "Xfire, Inc."],
    contributors: Array.from({ length: 4 }, (_, n) => longName(`Contributor_${n}_`)),
    url: `https://example.org/${"skins/".repeat(60)}${i}`,
    licence: {
      text: Array.from({ length: 12 }, (_, n) => `Clause ${n}: ${longName("Terms_")}`).join("\n"),
    },
  };
}

function skinLibrary(s: Stress) {
  const pkg = (i: number) => `pkg:${i.toString(16).padStart(64, "0")}`;
  const library = s.communities
    ? Array.from({ length: s.communities }, (_, i) => ({
        id: pkg(i + 1),
        name: longName(`Skin_${i}_`),
        importedAt: 1000 + i,
        credits: i % 2 ? null : skinCredits(i),
      }))
    : [];
  return {
    skins: [
      { id: "builtin:symbiosis", name: "Symbiosis", importedAt: null, credits: skinCredits(99) },
      { id: "builtin:default", name: "Default", importedAt: null, credits: null },
      ...library,
    ],
    active: "builtin:symbiosis",
    login: library.length ? pkg(2) : "builtin:symbiosis",
    trial: library.length
      ? { id: pkg(1), name: library[0].name, previous: "builtin:symbiosis", revertsAtMs: 0 }
      : null,
  };
}

/** `n` games' totals (plan E7.2): 64-char names, an Xfire icon, a key with no file, none. */
export function gameTotals(n: number) {
  const keys = ["xfire/8507", "slug/no-such-icon", null];
  return Array.from({ length: n }, (_, i) => ({
    gameId: 4000 + i,
    gameName: longName(`Game${i}_`),
    iconKey: keys[i % keys.length],
    totalSecs: 3600 * 9999 + i,
    lastPlayed: 1_000_000 + i,
  }));
}

export const FULL: Stress = { friends: 100, members: 100, messages: 40, communities: 12 };
export const EMPTY: Stress = { friends: 0, members: 0, messages: 0, communities: 0 };

/**
 * The handler source for one stress level, showing a skin on `base`
 * (plan E7.7.2's `get_skin`). `get_*`/`list_*` commands not
 * named answer an empty list, the rest `null`, as a window with no data
 * would see them.
 */
/// The longest bio the Host keeps (190 characters, plan E7.2).
export const FULL_BIO = ("W".repeat(60) + "\n" + "gg wp ".repeat(30)).slice(0, 190);

export function stressHandler(s: Stress, base: "dark" | "light" = "dark", welcome = false): string {
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
        gameDetectionEnabled: true, gameScanIntervalSecs: 30, readReceiptsEnabled: false,
        shareGameTotals: false, shareCurrentGame: false };
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
    case "group_dm_mark_read":
      return null;
    case "get_skin":
      return { id: "builtin:symbiosis", base: ${j(base)}, regions: [],
        tokensHref: ${j(`skin://localhost/${ME}/tokens.css`)} };
    case "list_skins": {
      // The Host's trial ends 15 s after it began.
      const lib = ${j(skinLibrary(s))};
      return { ...lib, trial: lib.trial && { ...lib.trial, revertsAtMs: Date.now() + 15000 } };
    }
    case "skin_draft_start":
      return { name: ${j(longName("Skin_").slice(0, 64))}, description: ${j(longName("Desc_").slice(0, 280))},
        author: ${j(longName("Author_").slice(0, 64))}, license: ${j("Free for any use. ".repeat(20))},
        credits: { authors: ${j(Array.from({ length: 16 }, (_, i) => longName(`Orig${i}_`).slice(0, 64)))},
          contributors: [${j(longName("Contrib_").slice(0, 64))}], url: ${j("https://example.org/" + "s".repeat(80))},
          licence: { text: ${j("Original licence terms. ".repeat(30))} } },
        base: ${j(base)}, contrast: "standard", tokens: {}, from: "builtin:symbiosis", regions: {} };
    case "validate_skin_draft": {
      // The skin editor (plan E7.7.5): a failing pair with its suggestion,
      // a refusal at its longest, and passing pairs with long grounds.
      const colour = (r, g, b) => ({ type: "color", data: { r, g, b, a: 255 } });
      const pair = (fg, ground, ratioX100, pass) => ({ rule: "1.4.3", fg, ground, ratioX100,
        requiredX100: 450, pass, apcaLcX10: -1075,
        suggestion: pass ? null : { token: fg, rgb: [52, 145, 216], ratioX100: 583 } });
      return {
        tokens: ["accent.text", "bg.dark", "bg.panel", "text.base", "status.online"].map(
          (id, i) => ({ id, css: "--color-xfire-" + id.replace(".", "-"), set: i === 0,
            value: colour(20 * i, 30 * i, 40 * i) })),
        pairs: [
          pair("accent.text", "bg.dark", 442, false),
          pair("text.base", "danger.fill 60.0% over bg.dark", 1590, true),
          pair("status.online", "accent.fill 12.0% over bg.panel", 812, true),
        ],
        failures: [${j("danger.fill: D1 hue 200.0°, chroma 0.050; danger must read red ".repeat(3))}],
        tokensCss: "",
        exportable: false,
        license: { text: args.draft.license },
        credits: { ...args.draft.credits, authors: [args.draft.author, ...args.draft.credits.authors] },
        regionSlots: [{ id: "scrollbar.track", carriesText: false },
          { id: "scrollbar.thumb-horizontal", carriesText: false }, { id: "statusbar", carriesText: true }],
      };
    }
    case "get_game_status":
      return null;
    case "get_game_totals":
      return ${j(gameTotals(s.friends ? 30 : 0))};
    case "get_bio":
    case "get_friend_bio":
      // A full 190-character bio (plan E7.2) with an unbroken 60-letter
      // word and a line break, or none.
      return ${j(s.friends ? FULL_BIO : null)};
    case "get_welcome_screen":
      // The Host's empty default (none set), or a set screen with the
      // longest description and featured channels.
      return ${j(welcome ? welcomeScreen() : { description: "", channels: [] })};
    case "get_onboarding_config":
      return { enabled: false, mode: "default", defaultChannels: [], questions: [],
        welcomeMessage: null, guideSteps: [] };
    case "get_safety_number":
      return ${j(safetyNumber(PEER))};
    case "get_active_call":
      return ${j(call)};
    case "get_network_status":
      // The Host's shape (commands/window.rs NetworkStatus), with some
      // bars lit so the indicator lays out lit and unlit bars.
      return { attachmentState: "attached_good", isAttached: true, publicInternetReady: true,
        hasRoute: true, mediaRoute: "available", canWrite: true, bars: 3,
        profileDhtKey: null, friendListDhtKey: null };
    default:
      return cmd.startsWith("get_") || cmd.startsWith("list_") ? [] : null;
  }
`;
}
