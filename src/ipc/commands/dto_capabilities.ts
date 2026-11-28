// The community capability map (plan E3.2 S9): mirrors
// `rekindle_types::dto::community_capability` and `MemberModeration`.
// The Host decides every gate and its reason; the window renders them.

/** The moderation actions against one member (Tier 1 `MemberModeration`):
 *  each the governance validator's verdict on the entry it writes, decided
 *  by the Host (plan E3.2 S9): `null` when allowed, else the Host's reason
 *  to show on the disabled action. */
export interface MemberModeration {
  kick: string | null;
  ban: string | null;
  timeout: string | null;
  manageRoles: string | null;
  /** Server mute; no voice-state reason before E3.4 S8b. */
  mute: string | null;
  /** Server deafen; no voice-state reason before E3.4 S8b. */
  deafen: string | null;
}

/** A community-window control gate (Tier 1 `CommunityCapability`). */
export type CommunityCapability =
  | "manageCommunity"
  | "manageChannels"
  | "setTopic"
  | "manageServers"
  | "manageRoles"
  | "kickMembers"
  | "banMembers"
  | "moderateMembers"
  | "createInvite"
  | "viewAuditLog"
  | "viewInsights"
  | "manageExpressions"
  | "createExpressions"
  | "manageEvents"
  | "createEvents"
  | "manageWelcome"
  | "manageAutomod"
  | "manageSecurity"
  | "offerSuccessor";

/** The capabilities we lack, each with the Host's reason (Tier 1
 *  `DeniedCapabilities`); a held capability is absent. */
export type DeniedCapabilities = Partial<Record<CommunityCapability, string>>;

/** A channel-scoped control gate (Tier 1 `ChannelCapability`), judged
 *  with the channel's overwrites. */
export type ChannelCapability =
  | "manageMessages"
  | "useSoundboard"
  | "createPublicThreads"
  | "sendInThreads"
  | "mentionEveryone"
  | "sendMessages"
  | "bypassSlowmode"
  | "requestToSpeak";

/** The channel controls we lack in one channel, each with the Host's
 *  reason (Tier 1 `ChannelDeniedCapabilities`); a held one is absent. */
export type ChannelDeniedCapabilities = Partial<Record<ChannelCapability, string>>;

/** One permission in the Host's catalogue (`rekindle_types::permissions`,
 *  contract PN1): its key, its label in the role editor, and its bit as a
 *  decimal string (a u64 is not JS-safe as a number). */
export interface PermissionEntry {
  key: string;
  label: string;
  bit: string;
}

/** A group of the catalogue, in the Host's order. */
export interface PermissionGroup {
  name: string;
  permissions: PermissionEntry[];
}

/** Every defined permission, once, grouped as the role editor shows them:
 *  the Host's table, never a shell copy (`get_permission_catalogue`). */
export interface PermissionCatalogue {
  groups: PermissionGroup[];
}
