import type { Role } from "../stores/community.store";

// Permission bit constants — mirrors Rust's `rekindle_protocol::dht::community::permissions_v2`.
// Uses Discord-aligned bit positions. BigInt is used throughout for correct 64-bit bitwise ops.

// ── General ──
export const CREATE_INSTANT_INVITE = 1n << 0n;
export const KICK_MEMBERS = 1n << 1n;
export const BAN_MEMBERS = 1n << 2n;
export const ADMINISTRATOR = 1n << 3n;
export const MANAGE_CHANNELS = 1n << 4n;
export const MANAGE_COMMUNITY = 1n << 5n;

// ── Text ──
export const ADD_REACTIONS = 1n << 6n;
export const VIEW_AUDIT_LOG = 1n << 7n;
export const PRIORITY_SPEAKER = 1n << 8n;
export const STREAM = 1n << 9n;
export const VIEW_CHANNEL = 1n << 10n;
export const SEND_MESSAGES = 1n << 11n;
// bit 12 reserved
export const MANAGE_MESSAGES = 1n << 13n;
export const EMBED_LINKS = 1n << 14n;
export const ATTACH_FILES = 1n << 15n;
export const READ_MESSAGE_HISTORY = 1n << 16n;
export const MENTION_EVERYONE = 1n << 17n;
export const USE_EXTERNAL_EMOJIS = 1n << 18n;
// bit 19 reserved

// ── Voice ──
export const CONNECT = 1n << 20n;
export const SPEAK = 1n << 21n;
export const MUTE_MEMBERS = 1n << 22n;
export const DEAFEN_MEMBERS = 1n << 23n;
export const MOVE_MEMBERS = 1n << 24n;
export const USE_VAD = 1n << 25n;

// ── Membership ──
export const CHANGE_NICKNAME = 1n << 26n;
export const MANAGE_NICKNAMES = 1n << 27n;
export const MANAGE_ROLES = 1n << 28n;
// bits 29-32 reserved

// ── Events ──
export const MANAGE_EVENTS = 1n << 33n;

// ── Threads ──
export const MANAGE_THREADS = 1n << 34n;
export const CREATE_PUBLIC_THREADS = 1n << 35n;
export const CREATE_PRIVATE_THREADS = 1n << 36n;
// bits 37-39 reserved

// ── Moderation ──
export const MODERATE_MEMBERS = 1n << 40n;
// bits 41-42 reserved

// ── Advanced ──
export const USE_APPLICATION_COMMANDS = 1n << 43n;
export const REQUEST_TO_SPEAK = 1n << 44n;
export const MANAGE_GUILD_EXPRESSIONS = 1n << 45n;
export const MANAGE_WEBHOOKS = 1n << 46n;
// bits 47-48 reserved
export const CREATE_GUILD_EXPRESSIONS = 1n << 49n;
// bits 50-52 reserved
export const SEND_VOICE_MESSAGES = 1n << 53n;
// bits 54-55 reserved
export const SEND_POLLS = 1n << 56n;
export const VIEW_CREATOR_MONETIZATION_ANALYTICS = 1n << 57n;

// All defined permission bits OR'd together — mirrors Rust's `Permissions::all()`.
export const ALL_PERMISSIONS =
  CREATE_INSTANT_INVITE | KICK_MEMBERS | BAN_MEMBERS | ADMINISTRATOR
  | MANAGE_CHANNELS | MANAGE_COMMUNITY | ADD_REACTIONS | VIEW_AUDIT_LOG
  | PRIORITY_SPEAKER | STREAM | VIEW_CHANNEL | SEND_MESSAGES
  | MANAGE_MESSAGES | EMBED_LINKS | ATTACH_FILES | READ_MESSAGE_HISTORY
  | MENTION_EVERYONE | USE_EXTERNAL_EMOJIS | CONNECT | SPEAK
  | MUTE_MEMBERS | DEAFEN_MEMBERS | MOVE_MEMBERS | USE_VAD
  | CHANGE_NICKNAME | MANAGE_NICKNAMES | MANAGE_ROLES
  | MANAGE_EVENTS | MANAGE_THREADS | CREATE_PUBLIC_THREADS | CREATE_PRIVATE_THREADS
  | MODERATE_MEMBERS | USE_APPLICATION_COMMANDS | REQUEST_TO_SPEAK
  | MANAGE_GUILD_EXPRESSIONS | MANAGE_WEBHOOKS | CREATE_GUILD_EXPRESSIONS
  | SEND_VOICE_MESSAGES | SEND_POLLS | VIEW_CREATOR_MONETIZATION_ANALYTICS;

/**
 * Check if a permission set includes a specific permission.
 * Administrators bypass all checks.
 */
export function hasPermission(perms: bigint, required: bigint): boolean {
  if ((perms & ADMINISTRATOR) !== 0n) return true;
  return (perms & required) === required;
}

/**
 * Check if permissions include the ADMINISTRATOR flag.
 */
export function isAdministrator(perms: bigint): boolean {
  return (perms & ADMINISTRATOR) !== 0n;
}

/**
 * Calculate base permissions for a member from their role IDs.
 * ORs together all role permissions for the member's roles.
 * This is a simplified client-side version for UI gating.
 *
 * Role.permissions is a string (serialized from Rust u64 to avoid JS Number precision loss).
 * Internally converts to BigInt for correct 64-bit bitwise operations.
 */
export function calculateBasePermissions(
  roleIds: number[],
  allRoles: Role[],
): bigint {
  let perms = 0n;
  for (const id of roleIds) {
    const role = allRoles.find((r) => r.id === id);
    if (role) {
      perms |= BigInt(role.permissions);
    }
  }
  return perms;
}

/**
 * Check if the user with the given highest role position can manage
 * a target with the given role position (must be strictly higher).
 */
export function canManageRole(
  myHighestPosition: number,
  targetPosition: number,
): boolean {
  return myHighestPosition > targetPosition;
}

/**
 * Get the highest role position from a set of role IDs.
 */
export function highestPosition(
  roleIds: number[],
  allRoles: Role[],
): number {
  let max = -1;
  for (const id of roleIds) {
    const role = allRoles.find((r) => r.id === id);
    if (role && role.position > max) {
      max = role.position;
    }
  }
  return max;
}

/**
 * Toggle a permission bit in a permission set.
 */
export function togglePermBit(current: bigint, bit: bigint): bigint {
  return current ^ bit;
}

/**
 * All permission constants grouped by category, for use in the role editor UI.
 */
export const PERMISSION_CATEGORIES = [
  {
    name: "General",
    permissions: [
      { key: "CREATE_INSTANT_INVITE", label: "Create Invite", value: CREATE_INSTANT_INVITE },
      { key: "MANAGE_CHANNELS", label: "Manage Channels", value: MANAGE_CHANNELS },
      { key: "MANAGE_COMMUNITY", label: "Manage Community", value: MANAGE_COMMUNITY },
      { key: "VIEW_AUDIT_LOG", label: "View Audit Log", value: VIEW_AUDIT_LOG },
      { key: "MANAGE_ROLES", label: "Manage Roles", value: MANAGE_ROLES },
      { key: "MANAGE_NICKNAMES", label: "Manage Nicknames", value: MANAGE_NICKNAMES },
      { key: "CHANGE_NICKNAME", label: "Change Nickname", value: CHANGE_NICKNAME },
      { key: "ADMINISTRATOR", label: "Administrator", value: ADMINISTRATOR },
    ],
  },
  {
    name: "Text",
    permissions: [
      { key: "VIEW_CHANNEL", label: "View Channels", value: VIEW_CHANNEL },
      { key: "SEND_MESSAGES", label: "Send Messages", value: SEND_MESSAGES },
      { key: "MANAGE_MESSAGES", label: "Manage Messages", value: MANAGE_MESSAGES },
      { key: "EMBED_LINKS", label: "Embed Links", value: EMBED_LINKS },
      { key: "ATTACH_FILES", label: "Attach Files", value: ATTACH_FILES },
      { key: "READ_MESSAGE_HISTORY", label: "Read History", value: READ_MESSAGE_HISTORY },
      { key: "ADD_REACTIONS", label: "Add Reactions", value: ADD_REACTIONS },
      { key: "MENTION_EVERYONE", label: "Mention Everyone", value: MENTION_EVERYONE },
      { key: "USE_EXTERNAL_EMOJIS", label: "External Emojis", value: USE_EXTERNAL_EMOJIS },
    ],
  },
  {
    name: "Voice",
    permissions: [
      { key: "CONNECT", label: "Connect", value: CONNECT },
      { key: "SPEAK", label: "Speak", value: SPEAK },
      { key: "STREAM", label: "Stream", value: STREAM },
      { key: "MUTE_MEMBERS", label: "Mute Members", value: MUTE_MEMBERS },
      { key: "DEAFEN_MEMBERS", label: "Deafen Members", value: DEAFEN_MEMBERS },
      { key: "MOVE_MEMBERS", label: "Move Members", value: MOVE_MEMBERS },
      { key: "USE_VAD", label: "Use Voice Detection", value: USE_VAD },
      { key: "PRIORITY_SPEAKER", label: "Priority Speaker", value: PRIORITY_SPEAKER },
      { key: "REQUEST_TO_SPEAK", label: "Request to Speak", value: REQUEST_TO_SPEAK },
    ],
  },
  {
    name: "Threads",
    permissions: [
      { key: "MANAGE_THREADS", label: "Manage Threads", value: MANAGE_THREADS },
      { key: "CREATE_PUBLIC_THREADS", label: "Create Public Threads", value: CREATE_PUBLIC_THREADS },
      { key: "CREATE_PRIVATE_THREADS", label: "Create Private Threads", value: CREATE_PRIVATE_THREADS },
    ],
  },
  {
    name: "Events",
    permissions: [
      { key: "MANAGE_EVENTS", label: "Manage Events", value: MANAGE_EVENTS },
    ],
  },
  {
    name: "Moderation",
    permissions: [
      { key: "KICK_MEMBERS", label: "Kick Members", value: KICK_MEMBERS },
      { key: "BAN_MEMBERS", label: "Ban Members", value: BAN_MEMBERS },
      { key: "MODERATE_MEMBERS", label: "Moderate Members", value: MODERATE_MEMBERS },
    ],
  },
  {
    name: "Advanced",
    permissions: [
      { key: "USE_APPLICATION_COMMANDS", label: "Application Commands", value: USE_APPLICATION_COMMANDS },
      { key: "MANAGE_GUILD_EXPRESSIONS", label: "Manage Expressions", value: MANAGE_GUILD_EXPRESSIONS },
      { key: "CREATE_GUILD_EXPRESSIONS", label: "Create Expressions", value: CREATE_GUILD_EXPRESSIONS },
      { key: "MANAGE_WEBHOOKS", label: "Manage Webhooks", value: MANAGE_WEBHOOKS },
      { key: "SEND_VOICE_MESSAGES", label: "Voice Messages", value: SEND_VOICE_MESSAGES },
      { key: "SEND_POLLS", label: "Send Polls", value: SEND_POLLS },
      { key: "VIEW_CREATOR_MONETIZATION_ANALYTICS", label: "View Analytics", value: VIEW_CREATOR_MONETIZATION_ANALYTICS },
    ],
  },
] as const;
