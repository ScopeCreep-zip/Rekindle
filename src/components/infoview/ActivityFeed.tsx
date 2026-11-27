import { type Component, For, Show, createMemo, createSignal } from "solid-js";
import { commands } from "../../ipc/commands";
import type { InfoviewAction } from "../../ipc/commands/dto_infoview";
import type { ActivityKind, ActivityListItem } from "../../ipc/commands/types";
import { communityState } from "../../stores/community.store";
import { formatDuration, formatRelativeTime } from "../../utils/formatting";
import { identityPinMismatchText } from "../../utils/identity_pin";

/** One older page (the Host's bound is 1–100). */
const PAGE = 20;

/**
 * The Infoview's Activity section (plan E7.15 IV6, design §2.3; Xfire's
 * "Activity Report"): the newest entries as the Host's section holds them
 * (replaced with each view it pushes), names as the Host holds them now.
 * View More is the Host's `activityMore{before}`: older rows page in below,
 * in place (`ActivityList`), not a website.
 */
const ActivityFeed: Component<{ entries: ActivityListItem[]; more: InfoviewAction | null }> = (props) => {
  const [older, setOlder] = createSignal<ActivityListItem[]>([]);
  // The cursor for the next older page: the section's first, then each page's.
  const [next, setNext] = createSignal<number | null | undefined>(undefined);
  const [error, setError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);

  const shown = createMemo(() => {
    const seen = new Set<number>();
    return [...props.entries, ...older()].filter((e) => !seen.has(e.id) && (seen.add(e.id), true));
  });
  const cursor = (): number | null => {
    const n = next();
    if (n !== undefined) return n;
    return props.more?.type === "activityMore" ? props.more.before : null;
  };

  async function viewMore(): Promise<void> {
    const before = cursor();
    if (before === null) return;
    setLoading(true);
    setError(null);
    try {
      const page = await commands.activityList(before, PAGE);
      setOlder((list) => [...list, ...page]);
      setNext(page.length === PAGE ? page[page.length - 1].id : null);
    } catch (e) {
      setError(`Activity not read: ${String(e)}`);
    } finally {
      setLoading(false);
    }
  }

  const gameNames = new Map<number, ReturnType<typeof createSignal<string | null>>>();
  function gameName(id: number): string {
    let entry = gameNames.get(id);
    if (!entry) {
      entry = createSignal<string | null>(null);
      gameNames.set(id, entry);
      const set = entry[1];
      commands.getGameName(id).then(set).catch(() => set(null));
    }
    return entry[0]() ?? `Game ${id}`;
  }

  const who = (e: ActivityListItem): string => e.nickname ?? e.displayName ?? "A friend";

  function text(e: ActivityListItem): string {
    const k: ActivityKind = e.kind;
    switch (k.type) {
      case "startedGame":
        return `${who(e)} started playing ${gameName(k.data.gameId)}${k.data.server ? ` on ${k.data.server}` : ""}`;
      case "stoppedGame":
        return `${who(e)} stopped playing ${gameName(k.data.gameId)} after ${formatDuration(k.data.seconds)}`;
      case "firstPlayed":
        return `${who(e)} has played ${gameName(k.data.gameId)} for the first time`;
      case "renamedSelf":
        return `${who(e)} changed their name from ${k.data.from} to ${k.data.to}`;
      case "statusMessage":
        return `${who(e)}: ${k.data.text}`;
      case "bioChanged":
        return `${who(e)} updated their profile`;
      case "nowFriends":
        return `You and ${who(e)} are now friends`;
      case "favouriteAdded":
        return `You added a new favourite: ${gameName(k.data.gameId)} server ${k.data.address}${k.data.label ? ` (${k.data.label})` : ""}`;
      case "joinedCommunity":
        return `You joined ${communityState.communities[k.data.community]?.name ?? "a community"}`;
    }
  }

  return (
    <section class="activity-feed infoview-section" aria-labelledby="activity-feed-title">
      <div class="activity-feed-title" id="activity-feed-title">
        Activity
      </div>
      <Show when={error()}>
        <div class="form-error" role="alert">
          {error()}
        </div>
      </Show>
      <Show when={shown().length > 0} fallback={<div class="activity-feed-empty">No activity yet.</div>}>
        <ul class="activity-feed-list">
          <For each={shown()}>
            {(e) => (
              <li class="activity-feed-item">
                <span class="activity-feed-text">{text(e)}</span>
                <time class="activity-feed-time" dateTime={new Date(e.at).toISOString()}>
                  {formatRelativeTime(e.at)}
                </time>
                <Show when={e.identityPinMismatch}>
                  <span class="activity-feed-mismatch" title={identityPinMismatchText(who(e))}>
                    Identity key mismatch
                  </span>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <Show when={cursor() !== null}>
        <button type="button" class="form-btn-secondary activity-feed-more" disabled={loading()} onClick={() => void viewMore()}>
          View More
        </button>
      </Show>
    </section>
  );
};

export default ActivityFeed;
