import type { Role } from "../stores/community.store";

// Permission bit constants — mirrors Rust's `rekindle_protocol::dht::community::permissions`.
// Uses Discord-aligned bit positions. Values above bit 31 must use BigInt-safe math.

// ── General ──
export const CREATE_INSTANT_INVITE = 1 << 0;
export const KICK_MEMBERS = 1 << 1;
export const BAN_MEMBERS = 1 << 2;
export const ADMINISTRATOR = 1 << 3;
export const MANAGE_CHANNELS = 1 << 4;
export const MANAGE_COMMUNITY = 1 << 5;

// ── Text ──
export const ADD_REACTIONS = 1 << 6;
export const VIEW_AUDIT_LOG = 1 << 7;
export const PRIORITY_SPEAKER = 1 << 8;
export const STREAM = 1 << 9;
export const VIEW_CHANNEL = 1 << 10;
export const SEND_MESSAGES = 1 << 11;
// bit 12 unused
export const MANAGE_MESSAGES = 1 << 13;
export const EMBED_LINKS = 1 << 14;
export const ATTACH_FILES = 1 << 15;
export const READ_MESSAGE_HISTORY = 1 << 16;
export const MENTION_EVERYONE = 1 << 17;
export const USE_EXTERNAL_EMOJIS = 1 << 18;

// ── Voice ──
export const CONNECT = 1 << 20;
export const SPEAK = 1 << 21;
export const MUTE_MEMBERS = 1 << 22;
export const DEAFEN_MEMBERS = 1 << 23;
export const MOVE_MEMBERS = 1 << 24;
export const USE_VAD = 1 << 25;

// ── Membership ──
export const CHANGE_NICKNAME = 1 << 26;
export const MANAGE_NICKNAMES = 1 << 27;
export const MANAGE_ROLES = 1 << 28;

// ── Moderation ──
// MODERATE_MEMBERS is at bit 40 in Rust (u64). JavaScript bitwise operators
// work on 32-bit ints so (1 << 40) === 0. We use a pre-computed value instead.
// 2^40 = 1099511627776, which is safely representable as a JS number (< 2^53).
export const MODERATE_MEMBERS = 2 ** 40;

// All defined permission bits OR'd together — mirrors Rust's `all_permissions()`.
// Must stay in sync with the Rust constant list.
export const ALL_PERMISSIONS =
  CREATE_INSTANT_INVITE | KICK_MEMBERS | BAN_MEMBERS | ADMINISTRATOR
  | MANAGE_CHANNELS | MANAGE_COMMUNITY | ADD_REACTIONS | VIEW_AUDIT_LOG
  | PRIORITY_SPEAKER | STREAM | VIEW_CHANNEL | SEND_MESSAGES
  | MANAGE_MESSAGES | EMBED_LINKS | ATTACH_FILES | READ_MESSAGE_HISTORY
  | MENTION_EVERYONE | USE_EXTERNAL_EMOJIS | CONNECT | SPEAK
  | MUTE_MEMBERS | DEAFEN_MEMBERS | MOVE_MEMBERS | USE_VAD
  | CHANGE_NICKNAME | MANAGE_NICKNAMES | MANAGE_ROLES
  // High bits (> 31) added via arithmetic since JS bitwise ops truncate to 32 bits
  + MODERATE_MEMBERS;

// The ADMINISTRATOR bit for quick checks
const ADMIN_BIT = ADMINISTRATOR;

/**
 * Check if a permission set includes a specific permission.
 * Administrators bypass all checks.
 */
export function hasPermission(perms: number, required: number): boolean {
  // Administrators have all permissions
  if ((perms & ADMIN_BIT) !== 0) return true;
  // For bits > 31 (like MODERATE_MEMBERS at bit 40), we can't use
  // bitwise AND because JS bitwise ops truncate to 32 bits. Instead
  // we test with floating-point math for those high bits.
  if (required > 0x7FFF_FFFF) {
    // High-bit permission — use modular arithmetic instead of bitwise
    return Math.floor(perms / required) % 2 === 1;
  }
  return (perms & required) === required;
}

/**
 * Check if permissions include the ADMINISTRATOR flag.
 */
export function isAdministrator(perms: number): boolean {
  return (perms & ADMIN_BIT) !== 0;
}

/**
 * Calculate base permissions for a member from their role IDs.
 * ORs together all role permissions for the member's roles.
 * This is a simplified client-side version for UI gating.
 *
 * Note: Rust sends permissions as u64 but JavaScript numbers can safely
 * represent integers up to 2^53. Permission bits go up to 40, so this is fine.
 * However, JS bitwise operators truncate to 32 bits, so we use addition
 * to combine permissions above bit 31.
 */
export function calculateBasePermissions(
  roleIds: number[],
  allRoles: Role[],
  isHosted?: boolean,
): number {
  // Community host (creator) always gets all permissions — like Discord's owner bypass
  if (isHosted) return ALL_PERMISSIONS;
  let perms = 0;
  for (const id of roleIds) {
    const role = allRoles.find((r) => r.id === id);
    if (role) {
      // Can't use |= for bits above 31, so we combine manually.
      // For low bits (0-30), bitwise OR is fine. For high bits, we add
      // them if not already present.
      const lowBits = (perms | role.permissions) & 0x7FFF_FFFF;
      const highPerms = role.permissions - (role.permissions & 0x7FFF_FFFF);
      const highCurrent = perms - (perms & 0x7FFF_FFFF);
      // Combine high bits: if either has the high bit set, keep it.
      // Since our only high bit is MODERATE_MEMBERS (2^40), simple max works.
      perms = lowBits + Math.max(highPerms, highCurrent);
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
] as const;
