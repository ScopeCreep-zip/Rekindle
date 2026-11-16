/** Our own removal as the backend reports it (list/info `removed`, or the
 *  live `kicked`/`banned` event naming us). */
export interface OwnRemovalView {
  banned: boolean;
  byName: string | null;
  reason?: string | null;
}

/**
 * "You were removed from <community> by <name>: <reason>", from the
 * backend's fields only: a part the backend does not name is left out,
 * never replaced by a placeholder (plan E3.2 S9, A/B R5-2).
 */
export function removalText(communityName: string, removed: OwnRemovalView): string {
  const what = removed.banned ? "banned" : "removed";
  const by = removed.byName ? ` by ${removed.byName}` : "";
  const why = removed.reason ? `: ${removed.reason}` : "";
  return `You were ${what} from ${communityName}${by}${why}`;
}
