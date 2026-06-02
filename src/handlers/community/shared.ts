import { createStore } from "solid-js/store";
import { commands } from "../../ipc/commands";
import { setCommunityState, communityState } from "../../stores/community.store";
import type { CommunityEvent as CommunityEventType, Role } from "../../stores/community.store";
import type { EventInfo } from "../../ipc/commands";

// Typing indicator state
export interface TypingUser {
  pseudonymKey: string;
  displayName: string;
}

const [typingUsersStore, setTypingUsers] = createStore<Record<string, TypingUser[]>>({});
const typingTimers: Record<string, number> = {};

export { typingUsersStore as typingUsers, setTypingUsers, typingTimers };

export function computeDisplayRoleName(roleIds: number[], roles: Role[]): string {
  const highestRole = roleIds
    .map((roleId) => roles.find((role) => role.id === roleId))
    .filter((role): role is Role => Boolean(role))
    .sort((a, b) => b.position - a.position)[0];
  return highestRole?.name ?? "member";
}

export function transformEvent(e: EventInfo): CommunityEventType {
  return {
    id: e.id,
    title: e.title,
    description: e.description,
    creatorPseudonym: e.creatorPseudonym,
    startTime: e.startTime,
    endTime: e.endTime,
    channelId: e.channelId,
    maxAttendees: e.maxAttendees,
    createdAt: e.createdAt,
    status: e.status as CommunityEventType["status"],
    rsvps: e.rsvps.map((r) => ({
      pseudonymKey: r.pseudonymKey,
      status: r.status as "going" | "maybe" | "declined",
    })),
    coverImageRef: e.coverImageRef,
    recurrence: e.recurrence,
    location: e.location,
  };
}

export async function refreshStageHandRaises(communityId: string, channelId: string): Promise<void> {
  const community = communityState.communities[communityId];
  const channel = community?.channels.find((item) => item.id === channelId);
  if (!channel || channel.type !== "stage") {
    return;
  }

  try {
    const pendingRequests = await commands.getStageHandRaises(communityId, channelId);
    setCommunityState("voiceChannels", channelId, (prev) => {
      const state = prev ?? { participants: [], mode: "mcu" as const, hostPseudonym: null };
      return { ...state, pendingRequests };
    });
  } catch (e) {
    console.error("Failed to load stage hand raises:", e);
  }
}
