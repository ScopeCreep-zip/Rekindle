import type { Role } from "../stores/community.store";

export const VIEW_CHANNELS = 1n << 0n;
export const MANAGE_CHANNELS = 1n << 1n;
export const MANAGE_ROLES = 1n << 2n;
export const MANAGE_COMMUNITY = 1n << 3n;
export const CREATE_INVITES = 1n << 4n;
export const KICK_MEMBERS = 1n << 5n;
export const BAN_MEMBERS = 1n << 6n;
export const MODERATE_MEMBERS = 1n << 7n;
export const MANAGE_NICKNAMES = 1n << 8n;
export const MANAGE_EXPRESSIONS = 1n << 9n;
export const VIEW_AUDIT_LOG = 1n << 10n;
export const VIEW_INSIGHTS = 1n << 11n;
export const CREATE_EXPRESSIONS = 1n << 12n;

export const SEND_MESSAGES = 1n << 16n;
export const EMBED_LINKS = 1n << 17n;
export const ATTACH_FILES = 1n << 18n;
export const ADD_REACTIONS = 1n << 19n;
export const MENTION_EVERYONE = 1n << 20n;
export const MANAGE_MESSAGES = 1n << 21n;
export const READ_HISTORY = 1n << 22n;
export const SEND_TTS = 1n << 23n;
export const USE_EXTERNAL_EMOJIS = 1n << 24n;
export const USE_EXTERNAL_STICKERS = 1n << 25n;
export const PIN_MESSAGES = 1n << 26n;
export const SEND_VOICE_MESSAGES = 1n << 27n;
export const SEND_POLLS = 1n << 28n;
export const BYPASS_SLOWMODE = 1n << 29n;

export const CONNECT = 1n << 32n;
export const SPEAK = 1n << 33n;
export const MUTE_MEMBERS = 1n << 34n;
export const DEAFEN_MEMBERS = 1n << 35n;
export const MOVE_MEMBERS = 1n << 36n;
export const USE_VOICE_ACTIVITY = 1n << 37n;
export const PRIORITY_SPEAKER = 1n << 38n;
export const USE_SOUNDBOARD = 1n << 39n;
export const USE_EXTERNAL_SOUNDS = 1n << 40n;
export const REQUEST_TO_SPEAK = 1n << 41n;
export const STREAM = 1n << 42n;

export const MANAGE_THREADS = 1n << 44n;
export const CREATE_PUBLIC_THREADS = 1n << 45n;
export const CREATE_PRIVATE_THREADS = 1n << 46n;
export const SEND_IN_THREADS = 1n << 47n;

export const MANAGE_EVENTS = 1n << 48n;
export const CREATE_EVENTS = 1n << 49n;
export const ADMINISTRATOR = 1n << 50n;

export const CREATE_INSTANT_INVITE = CREATE_INVITES;
export const VIEW_CHANNEL = VIEW_CHANNELS;
export const READ_MESSAGE_HISTORY = READ_HISTORY;
export const CHANGE_NICKNAME = 0n;
export const USE_VAD = USE_VOICE_ACTIVITY;

export function permissionBits(value: string | bigint | null | undefined): bigint {
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

export function isAdministrator(permissions: string | bigint | null | undefined): boolean {
  return (permissionBits(permissions) & ADMINISTRATOR) !== 0n;
}

export function calculateBasePermissions(roleIds: number[], allRoles: Role[]): bigint {
  let perms = 0n;
  for (const id of roleIds) {
    const role = allRoles.find((r) => r.id === id);
    if (role) perms |= permissionBits(role.permissions);
  }
  return perms;
}

export function canManageRole(myHighestPosition: number, targetPosition: number): boolean {
  return myHighestPosition > targetPosition;
}

export function highestPosition(roleIds: number[], allRoles: Role[]): number {
  let max = -1;
  for (const id of roleIds) {
    const role = allRoles.find((r) => r.id === id);
    if (role && role.position > max) max = role.position;
  }
  return max;
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
