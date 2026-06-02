import { invoke } from "../invoke";
import type {
  CreateEventRequest, EventInfo, ExclusionGroupEdit, Message,
} from "./types";

export const governanceCommands = {
  // Reactions
  addReaction: (communityId: string, channelId: string, messageId: string, emoji: string) =>
    invoke<void>("add_reaction", { communityId, channelId, messageId, emoji }),
  removeReaction: (communityId: string, channelId: string, messageId: string, emoji: string) =>
    invoke<void>("remove_reaction", { communityId, channelId, messageId, emoji }),

  // Pins
  pinMessage: (communityId: string, channelId: string, messageId: string) =>
    invoke<void>("pin_message", { communityId, channelId, messageId }),
  unpinMessage: (communityId: string, channelId: string, messageId: string) =>
    invoke<void>("unpin_message", { communityId, channelId, messageId }),
  getChannelPins: (communityId: string, channelId: string) =>
    invoke<{ messageId: string; channelId: string; pinnedBy: string; pinnedAt: number }[]>(
      "get_channel_pins",
      { communityId, channelId },
    ),

  sendChannelTyping: (communityId: string, channelId: string) =>
    invoke<void>("send_channel_typing", { communityId, channelId }),

  updateCommunityPresence: (communityId: string, status: string, gameName?: string, gameId?: number, elapsedSeconds?: number, serverAddress?: string) =>
    invoke<void>("update_community_presence", { communityId, status, gameName, gameId, elapsedSeconds, serverAddress }),

  // Audit log
  getAuditLog: (communityId: string, beforeTimestamp?: number, limit: number = 50) =>
    invoke<{ action: string; actorPseudonym: string; target: string | null; details: string | null; timestamp: number }[]>(
      "get_audit_log",
      { communityId, beforeTimestamp, limit },
    ),

  // Categories
  createCategory: (communityId: string, name: string) =>
    invoke<{ categoryId: string }>("create_category", { communityId, name }),
  deleteCategory: (communityId: string, categoryId: string) =>
    invoke<void>("delete_category", { communityId, categoryId }),
  renameCategory: (communityId: string, categoryId: string, newName: string) =>
    invoke<void>("rename_category", { communityId, categoryId, newName }),
  moveChannel: (communityId: string, channelId: string, categoryId: string | null) =>
    invoke<void>("move_channel", { communityId, channelId, categoryId }),
  reorderCategories: (communityId: string, categoryIds: string[]) =>
    invoke<void>("reorder_categories", { communityId, categoryIds }),
  setChannelTopic: (communityId: string, channelId: string, topic: string) =>
    invoke<void>("set_channel_topic", { communityId, channelId, topic }),
  setChannelForumTags: (communityId: string, channelId: string, forumTags: string[]) =>
    invoke<void>("set_channel_forum_tags", { communityId, channelId, forumTags }),
  reorderChannels: (communityId: string, channelIds: string[]) =>
    invoke<void>("reorder_channels", { communityId, channelIds }),

  // Community invites
  createCommunityInvite: (communityId: string, maxUses?: number, expiresInSeconds?: number) =>
    invoke<{ code: string; governanceKey: string }>("create_community_invite", { communityId, maxUses: maxUses ?? null, expiresInSeconds: expiresInSeconds ?? null }),
  revokeCommunityInvite: (communityId: string, codeHash: string) =>
    invoke<void>("revoke_community_invite", { communityId, codeHash }),
  listCommunityInvites: (communityId: string) =>
    invoke<{ codeHash: string; createdBy: string; maxUses: number | null; uses: number; expiresAt: number | null; createdAt: number; code?: string }[]>(
      "list_community_invites", { communityId }
    ),

  // Roles
  getRoles: (communityId: string) =>
    invoke<{ id: number; name: string; color: number; permissions: number; position: number; hoist: boolean; mentionable: boolean; selfAssignable?: boolean; exclusionGroup?: string }[]>(
      "get_roles", { communityId },
    ),
  createRole: (
    communityId: string,
    name: string,
    color: number,
    permissions: string,
    hoist: boolean,
    mentionable: boolean,
    selfAssignable: boolean,
    /** Architecture §19.4 — when set, the member can hold at most one role per group. */
    exclusionGroup?: string,
  ) =>
    invoke<number>("create_role", {
      communityId,
      name,
      color,
      permissions,
      hoist,
      mentionable,
      selfAssignable,
      exclusionGroup: exclusionGroup ?? null,
    }),
  editRole: (
    communityId: string,
    roleId: number,
    name: string | null,
    color: number | null,
    permissions: string | null,
    position: number | null,
    hoist: boolean | null,
    mentionable: boolean | null,
    selfAssignable: boolean | null,
    /**
     * Architecture §19.4 — pass `{ kind: "set", value: "..." }` to set,
     * `{ kind: "clear" }` to clear, or omit to leave unchanged.
     */
    exclusionGroup?: ExclusionGroupEdit,
  ) =>
    invoke<void>("edit_role", {
      communityId,
      roleId,
      name,
      color,
      permissions,
      position,
      hoist,
      mentionable,
      selfAssignable,
      exclusionGroup: exclusionGroup ?? null,
    }),
  deleteRole: (communityId: string, roleId: number) =>
    invoke<void>("delete_role", { communityId, roleId }),
  assignRole: (communityId: string, pseudonymKey: string, roleId: number) =>
    invoke<void>("assign_role", { communityId, pseudonymKey, roleId }),
  unassignRole: (communityId: string, pseudonymKey: string, roleId: number) =>
    invoke<void>("unassign_role", { communityId, pseudonymKey, roleId }),
  selfAssignRole: (communityId: string, roleId: number) =>
    invoke<void>("self_assign_role", { communityId, roleId }),
  selfUnassignRole: (communityId: string, roleId: number) =>
    invoke<void>("self_unassign_role", { communityId, roleId }),
  timeoutMember: (communityId: string, pseudonymKey: string, durationSeconds: number, reason: string | null) =>
    invoke<void>("timeout_member", { communityId, pseudonymKey, durationSeconds, reason }),
  removeTimeout: (communityId: string, pseudonymKey: string) =>
    invoke<void>("remove_timeout", { communityId, pseudonymKey }),
  setChannelOverwrite: (communityId: string, channelId: string, targetType: string, targetId: string, allow: number, deny: number) =>
    invoke<void>("set_channel_overwrite", { communityId, channelId, targetType, targetId, allow, deny }),
  deleteChannelOverwrite: (communityId: string, channelId: string, targetType: string, targetId: string) =>
    invoke<void>("delete_channel_overwrite", { communityId, channelId, targetType, targetId }),
  setSlowmode: (communityId: string, channelId: string, seconds: number) =>
    invoke<void>("set_slowmode", { communityId, channelId, seconds }),

  // Threads
  /**
   * Architecture §32 Phase 6 W19 — `autoArchiveSeconds` must be one of
   * 3600 (1h), 86400 (24h), 259200 (3d), or 604800 (7d). Omit to let
   * the backend pick the per-thread-type default.
   */
  createThread: (
    communityId: string,
    channelId: string,
    name: string,
    starterMessageId: string,
    forumTag?: string | null,
    autoArchiveSeconds?: number,
  ) =>
    invoke<string>("create_thread", {
      communityId,
      channelId,
      name,
      starterMessageId,
      forumTag: forumTag ?? null,
      autoArchiveSeconds: autoArchiveSeconds ?? null,
    }),
  getChannelThreads: (communityId: string, channelId: string) =>
    invoke<{ id: string; channelId: string; name: string; starterMessageId: string; creatorPseudonym: string; forumTag?: string | null; createdAt: number; archived: boolean; autoArchiveSeconds: number; lastMessageAt: number; messageCount: number }[]>(
      "get_channel_threads", { communityId, channelId }),
  getActiveThreads: (communityId: string, channelId: string) =>
    invoke<{ id: string; channelId: string; name: string; starterMessageId: string; creatorPseudonym: string; forumTag?: string | null; createdAt: number; archived: boolean; autoArchiveSeconds: number; lastMessageAt: number; messageCount: number }[]>(
      "get_active_threads", { communityId, channelId }),
  sendThreadMessage: (communityId: string, threadId: string, body: string) =>
    invoke<void>("send_thread_message", { communityId, threadId, body }),
  getThreadMessages: (communityId: string, threadId: string, limit: number, beforeTimestamp?: number) =>
    invoke<Message[]>("get_thread_messages", { communityId, threadId, limit, beforeTimestamp: beforeTimestamp ?? null }),
  archiveThread: (communityId: string, threadId: string) =>
    invoke<void>("archive_thread", { communityId, threadId }),
  unarchiveThread: (communityId: string, threadId: string) =>
    invoke<void>("unarchive_thread", { communityId, threadId }),

  // Community Events (architecture §21)
  createEvent: (communityId: string, request: CreateEventRequest) =>
    invoke<string>("create_event", { communityId, request }),
  editEvent: (communityId: string, eventId: string, title?: string, description?: string, startTime?: number, endTime?: number, channelId?: string, maxAttendees?: number) =>
    invoke<void>("edit_event", { communityId, eventId, title: title ?? null, description: description ?? null, startTime: startTime ?? null, endTime: endTime ?? null, channelId: channelId ?? null, maxAttendees: maxAttendees ?? null }),
  deleteEvent: (communityId: string, eventId: string) =>
    invoke<void>("delete_event", { communityId, eventId }),
  cancelEvent: (communityId: string, eventId: string) =>
    invoke<void>("cancel_event", { communityId, eventId }),
  rsvpEvent: (communityId: string, eventId: string, status: string) =>
    invoke<void>("rsvp_event", { communityId, eventId, status }),
  setEventRsvp: (communityId: string, eventId: string, status: string) =>
    invoke<void>("set_event_rsvp", { communityId, eventId, status }),
  listEventAttendees: (communityId: string, eventId: string) =>
    invoke<{ pseudonymKey: string; status: string }[]>("list_event_attendees", { communityId, eventId }),
  getEvents: (communityId: string) =>
    invoke<EventInfo[]>("get_events", { communityId }),

  // Game Servers
  addGameServer: (communityId: string, gameId: string, label: string, address: string) =>
    invoke<string>("add_game_server", { communityId, gameId, label, address }),
  removeGameServer: (communityId: string, serverId: string) =>
    invoke<void>("remove_game_server", { communityId, serverId }),
  getGameServers: (communityId: string) =>
    invoke<{ id: string; gameId: string; label: string; address: string; addedBy: string; createdAt: number }[]>(
      "get_game_servers", { communityId }),

  // Unread tracking
  markChannelRead: (communityId: string, channelId: string, lastMessageId: string) =>
    invoke<void>("mark_channel_read", { communityId, channelId, lastMessageId }),
  getUnreadCounts: (communityId: string) =>
    invoke<{ channelId: string; unreadCount: number }[]>("get_unread_counts", { communityId }),
};
