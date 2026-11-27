const VIEW_CHANNELS = 1n << 0n;
export const MANAGE_CHANNELS = 1n << 1n;
export const MANAGE_ROLES = 1n << 2n;
export const MANAGE_COMMUNITY = 1n << 3n;
export const CREATE_INVITES = 1n << 4n;
export const KICK_MEMBERS = 1n << 5n;
export const BAN_MEMBERS = 1n << 6n;
const MODERATE_MEMBERS = 1n << 7n;
export const MANAGE_EXPRESSIONS = 1n << 9n;
export const VIEW_AUDIT_LOG = 1n << 10n;
const VIEW_INSIGHTS = 1n << 11n;
const CREATE_EXPRESSIONS = 1n << 12n;

export const SEND_MESSAGES = 1n << 16n;
const EMBED_LINKS = 1n << 17n;
const ATTACH_FILES = 1n << 18n;
const ADD_REACTIONS = 1n << 19n;
export const MENTION_EVERYONE = 1n << 20n;
export const MANAGE_MESSAGES = 1n << 21n;
const READ_HISTORY = 1n << 22n;
const USE_EXTERNAL_EMOJIS = 1n << 24n;
const PIN_MESSAGES = 1n << 26n;
const SEND_POLLS = 1n << 28n;
export const BYPASS_SLOWMODE = 1n << 29n;

const CONNECT = 1n << 32n;
const SPEAK = 1n << 33n;
const MUTE_MEMBERS = 1n << 34n;
const DEAFEN_MEMBERS = 1n << 35n;
const MOVE_MEMBERS = 1n << 36n;
const USE_VOICE_ACTIVITY = 1n << 37n;
export const USE_SOUNDBOARD = 1n << 39n;
export const REQUEST_TO_SPEAK = 1n << 41n;
const STREAM = 1n << 42n;

const MANAGE_THREADS = 1n << 44n;
const CREATE_PUBLIC_THREADS = 1n << 45n;
const CREATE_PRIVATE_THREADS = 1n << 46n;
const SEND_IN_THREADS = 1n << 47n;

const MANAGE_EVENTS = 1n << 48n;
const CREATE_EVENTS = 1n << 49n;
const ADMINISTRATOR = 1n << 50n;

function permissionBits(value: string | bigint | null | undefined): bigint {
  if (typeof value === "bigint") return value;
  return BigInt(value || "0");
}

export function hasPermission(
  permissions: string | bigint | null | undefined,
  required: bigint,
): boolean {
  const perms = permissionBits(permissions);
  if ((perms & ADMINISTRATOR) !== 0n) return true;
  return (perms & required) === required;
}

/** What we may do in a community, as the backend computed it from the
 *  merged governance (plan E3.2 S9): `channelId`'s effective permissions
 *  (base plus its overwrites) when given and known, else the base. The
 *  frontend computes no policy from roles. */
export function servedPermissions(
  community: { myPermissions: string; myChannelPermissions: Record<string, string> } | null | undefined,
  channelId?: string | null,
): bigint {
  if (!community) return 0n;
  const inChannel = channelId ? community.myChannelPermissions[channelId] : undefined;
  return permissionBits(inChannel ?? community.myPermissions);
}

export function togglePermBit(current: bigint, bit: bigint): bigint {
  return current ^ bit;
}

export const PERMISSION_CATEGORIES = [
  {
    name: "General",
    permissions: [
      { key: "VIEW_CHANNELS", label: "View Channels", value: VIEW_CHANNELS },
      { key: "MANAGE_CHANNELS", label: "Manage Channels", value: MANAGE_CHANNELS },
      { key: "MANAGE_ROLES", label: "Manage Roles", value: MANAGE_ROLES },
      { key: "MANAGE_COMMUNITY", label: "Manage Community", value: MANAGE_COMMUNITY },
      { key: "CREATE_INVITES", label: "Create Invites", value: CREATE_INVITES },
      { key: "KICK_MEMBERS", label: "Kick Members", value: KICK_MEMBERS },
      { key: "BAN_MEMBERS", label: "Ban Members", value: BAN_MEMBERS },
      { key: "MODERATE_MEMBERS", label: "Moderate Members", value: MODERATE_MEMBERS },
      { key: "VIEW_AUDIT_LOG", label: "View Audit Log", value: VIEW_AUDIT_LOG },
      { key: "ADMINISTRATOR", label: "Administrator", value: ADMINISTRATOR },
    ],
  },
  {
    name: "Text",
    permissions: [
      { key: "SEND_MESSAGES", label: "Send Messages", value: SEND_MESSAGES },
      { key: "EMBED_LINKS", label: "Embed Links", value: EMBED_LINKS },
      { key: "ATTACH_FILES", label: "Attach Files", value: ATTACH_FILES },
      { key: "ADD_REACTIONS", label: "Add Reactions", value: ADD_REACTIONS },
      { key: "MENTION_EVERYONE", label: "Mention Everyone", value: MENTION_EVERYONE },
      { key: "MANAGE_MESSAGES", label: "Manage Messages", value: MANAGE_MESSAGES },
      { key: "READ_HISTORY", label: "Read History", value: READ_HISTORY },
      { key: "PIN_MESSAGES", label: "Pin Messages", value: PIN_MESSAGES },
      { key: "SEND_POLLS", label: "Send Polls", value: SEND_POLLS },
    ],
  },
  {
    name: "Voice",
    permissions: [
      { key: "CONNECT", label: "Connect", value: CONNECT },
      { key: "SPEAK", label: "Speak", value: SPEAK },
      { key: "MUTE_MEMBERS", label: "Mute Members", value: MUTE_MEMBERS },
      { key: "DEAFEN_MEMBERS", label: "Deafen Members", value: DEAFEN_MEMBERS },
      { key: "MOVE_MEMBERS", label: "Move Members", value: MOVE_MEMBERS },
      { key: "USE_VOICE_ACTIVITY", label: "Use Voice Activity", value: USE_VOICE_ACTIVITY },
      { key: "REQUEST_TO_SPEAK", label: "Request to Speak", value: REQUEST_TO_SPEAK },
      { key: "STREAM", label: "Stream", value: STREAM },
    ],
  },
  {
    name: "Threads",
    permissions: [
      { key: "MANAGE_THREADS", label: "Manage Threads", value: MANAGE_THREADS },
      { key: "CREATE_PUBLIC_THREADS", label: "Create Public Threads", value: CREATE_PUBLIC_THREADS },
      { key: "CREATE_PRIVATE_THREADS", label: "Create Private Threads", value: CREATE_PRIVATE_THREADS },
      { key: "SEND_IN_THREADS", label: "Send In Threads", value: SEND_IN_THREADS },
    ],
  },
  {
    name: "Events",
    permissions: [
      { key: "MANAGE_EVENTS", label: "Manage Events", value: MANAGE_EVENTS },
      { key: "CREATE_EVENTS", label: "Create Events", value: CREATE_EVENTS },
    ],
  },
] as const;
