import type { CommunityCapability } from "../ipc/commands/dto_capabilities";
import type { Community } from "../stores/community.store";

/**
 * Why we may not do `cap` in `c`, in the Host's words, or null when we may
 * (never-hide: the control is drawn either way, disabled with this reason).
 */
export function deniedFor(c: Community | undefined, cap: CommunityCapability): string | null {
  return c?.denied?.[cap] ?? null;
}
