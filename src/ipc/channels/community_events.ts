import type { CommunityVideoEvent } from "./community_video_events";

/**
 * Events on the `community-event` channel.
 *
 * Mid-migration, this channel carries **two** shapes, exactly as
 * `chat-event` does:
 *
 * 1. {@link CommunityEvent} — the desktop's `{ type, data }` envelope.
 * 2. {@link CommunitySubscriptionEvent} — the daemon vocabulary the CLI
 *    also consumes, externally tagged. The membership family has moved
 *    across; governance, social, crypto, voice signalling and the rest
 *    have not yet.
 *
 * {@link isLegacyCommunityEvent} discriminates the two.
 */

export type CommunityEvent =
  | {
      // Architecture §18.4 — eager-fetched expression bytes have landed.
      // Frontend should re-pull list_expressions for this community so
      // the picker re-renders with the resolved inline_data_base64.
      type: "expressionAssetReady";
      data: { communityId: string; expressionId: string };
    }
  | {
      type: "raidDetected";
      data: {
        communityId: string;
        joinsInWindow: number;
        maxJoinsPerInterval: number;
        joinIntervalSeconds: number;
      };
    }
  | {
      // Architecture §28.8 — sender pre-fetched OpenGraph metadata.
      type: "linkPreviewReceived";
      data: {
        communityId: string;
        senderPseudonym: string;
        channelId: string;
        messageId: string;
        url: string;
        title?: string;
        description?: string;
        imageUrl?: string;
        siteName?: string;
        fetchedAt: number;
      };
    }
  | {
      type: "mekRotated";
      data: { communityId: string; channelId?: string; newGeneration: number };
    }
  | {
      type: "rolesChanged";
      data: {
        communityId: string;
        roles: { id: number; name: string; color: number; permissions: string; position: number; hoist: boolean; mentionable: boolean; selfAssignable?: boolean }[];
      };
    }
  | {
      type: "channelOverwriteChanged";
      data: { communityId: string; channelId: string };
    }
  | {
      type: "governanceUpdated";
      data: { communityId: string };
    }
  | {
      type: "messageEdited";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
        newBody: string;
        editedAt: number;
      };
    }
  | {
      type: "messageDeleted";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
      };
    }
  | {
      type: "reactionAdded";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
        emoji: string;
        reactorPseudonym: string;
      };
    }
  | {
      type: "reactionRemoved";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
        emoji: string;
        reactorPseudonym: string;
      };
    }
  | {
      type: "messagePinned";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
        pinnedBy: string;
      };
    }
  | {
      type: "messageUnpinned";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
      };
    }
  | {
      type: "channelMessageDelivered";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
      };
    }
  | {
      type: "channelMessageDeliveryFailed";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
      };
    }
  | {
      type: "channelTyping";
      data: {
        communityId: string;
        channelId: string;
        pseudonymKey: string;
      };
    }
  | {
      type: "memberPresenceChanged";
      data: {
        communityId: string;
        pseudonymKey: string;
        status: string;
        gameName?: string;
        gameId?: number;
        elapsedSeconds?: number;
        serverAddress?: string;
      };
    }
  | {
      type: "autoModAlert";
      data: {
        communityId: string;
        channelId: string;
        messageId: string;
        ruleName: string;
      };
    }
  | {
      type: "threadCreated";
      data: {
        communityId: string;
        thread: {
          id: string;
          channelId: string;
          name: string;
          starterMessageId: string;
          creatorPseudonym: string;
          forumTag?: string | null;
          createdAt: number;
          archived: boolean;
          autoArchiveSeconds: number;
          lastMessageAt: number;
          messageCount: number;
        };
      };
    }
  | {
      type: "threadMessageReceived";
      data: {
        communityId: string;
        threadId: string;
        messageId: string;
        senderPseudonym: string;
        body: string;
        timestamp: number;
        replyToId: string | null;
      };
    }
  | {
      type: "threadArchived";
      data: {
        communityId: string;
        threadId: string;
        archived: boolean;
      };
    }
  | {
      type: "eventCreated";
      data: {
        communityId: string;
        event: {
          id: string;
          title: string;
          description: string;
          creatorPseudonym: string;
          startTime: number;
          endTime: number | null;
          channelId: string | null;
          maxAttendees: number | null;
          createdAt: number;
          status: string;
          rsvps: { pseudonymKey: string; status: string }[];
        };
      };
    }
  | {
      type: "eventUpdated";
      data: {
        communityId: string;
        event: {
          id: string;
          title: string;
          description: string;
          creatorPseudonym: string;
          startTime: number;
          endTime: number | null;
          channelId: string | null;
          maxAttendees: number | null;
          createdAt: number;
          status: string;
          rsvps: { pseudonymKey: string; status: string }[];
        };
      };
    }
  | {
      type: "eventDeleted";
      data: {
        communityId: string;
        eventId: string;
      };
    }
  | {
      type: "eventRsvpChanged";
      data: {
        communityId: string;
        eventId: string;
        pseudonymKey: string;
        status: string;
      };
    }
  | {
      type: "gameServerAdded";
      data: {
        communityId: string;
        server: {
          id: string;
          gameId: string;
          label: string;
          address: string;
          addedBy: string;
          createdAt: number;
        };
      };
    }
  | {
      type: "gameServerRemoved";
      data: {
        communityId: string;
        serverId: string;
      };
    }
  | {
      type: "eventReminder";
      data: {
        communityId: string;
        eventId: string;
        title: string;
        minutesUntilStart: number;
      };
    }
  | {
      type: "channelsUpdated";
      data: {
        communityId: string;
        channels: { id: string; name: string; channelType: string; categoryId?: string; topic?: string; slowmodeSeconds?: number }[];
        categories: { id: string; name: string; sortOrder: number }[];
      };
    }
  | {
      type: "inviteCreated";
      data: {
        communityId: string;
        codeHash: string;
        createdBy: string;
        maxUses: number | null;
        uses: number;
        expiresAt: number | null;
        createdAt: number;
      };
    }
  | {
      type: "inviteRevoked";
      data: { communityId: string; codeHash: string };
    }
  | {
      type: "inviteUsed";
      data: { communityId: string; codeHash: string; newUseCount: number };
    }
  | {
      type: "systemMessage";
      data: {
        communityId: string;
        body: string;
        timestamp: number;
      };
    }
  | {
      type: "raidAlert";
      data: {
        communityId: string;
        active: boolean;
      };
    }
  | {
      type: "channelLockdown";
      data: {
        communityId: string;
        locked: boolean;
      };
    }
  | {
      type: "syncComplete";
      data: {
        communityId: string;
        channelId: string;
        messageCount: number;
      };
    }
  | {
      type: "communityUpdated";
      data: {
        communityId: string;
        name: string | null;
        description: string | null;
        iconHash: string | null;
        bannerHash: string | null;
      };
    }
  | {
      type: "attachmentDownloaded";
      data: {
        communityId: string;
        channelId: string;
        attachmentId: string;
        localPath: string;
      };
    }
  | CommunityVideoEvent;
