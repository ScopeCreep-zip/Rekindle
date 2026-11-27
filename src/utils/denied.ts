import type { ChannelCapability, CommunityCapability } from "../ipc/commands/dto_capabilities";
import type { Community } from "../stores/community.store";

/**
 * Why we may not do `cap` in `c`, in the Host's words, or null when we may
 * (never-hide: the control is drawn either way, disabled with this reason).
 */
export function deniedFor(c: Community | undefined, cap: CommunityCapability): string | null {
  return c?.denied?.[cap] ?? null;
}

/**
 * Why we may not do the channel-scoped `cap` in channel `channelId` of `c`
 * (judged with the channel's overwrites by the Host), or null when we may.
 */
export function channelDeniedFor(c: Community | null | undefined, channelId: string | null | undefined, cap: ChannelCapability): string | null {
  if (!c || !channelId) return null;
  return c.channels.find((ch) => ch.id === channelId)?.denied?.[cap] ?? null;
}
