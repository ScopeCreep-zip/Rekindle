import { Component, For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { commands } from "../../ipc/commands";
import { subscribeActivity } from "../../ipc/channels";
import type { ActivityKind, ActivityListItem } from "../../ipc/commands/types";
import { communityState } from "../../stores/community.store";
import { formatDuration, formatRelativeTime } from "../../utils/formatting";
import { identityPinMismatchText } from "../../utils/identity_pin";

/** One page of the feed (the Host's bound is 1–100). */
const PAGE = 20;

/**
 * The Infoview's Activity section (plan E7.15 IV6, design §2.3; Xfire's
 * "Activity Report"): the local feed the Host keeps, newest first, the
 * names as the Host holds them now. A page loads on mount, each
 * `ActivityAppended` adds its entries at the top, and View More pages
 * older rows (`ActivityList{before}`), in place: it is not a website.
 * The window decides nothing; it renders what the Host said.
 */
const ActivityFeed: Component = () => {
  const [entries, setEntries] = createSignal<ActivityListItem[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);
  // Whether the last page was full: the Host may hold older rows.
  const [more, setMore] = createSignal(false);
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

  function merge(incoming: ActivityListItem[], older: boolean): void {
    setEntries((list) => {
      const seen = new Set(list.map((e) => e.id));
      const fresh = incoming.filter((e) => !seen.has(e.id));
      const all = older ? [...list, ...fresh] : [...fresh, ...list];
      return all.sort((a, b) => b.id - a.id);
    });
  }

  async function load(older: boolean): Promise<void> {
    setLoading(true);
    setError(null);
    try {
      const list = entries();
      const before = older && list.length > 0 ? list[list.length - 1].id : null;
      const page = await commands.activityList(before, PAGE);
      merge(page, older);
      setMore(page.length === PAGE);
    } catch (e) {
      setError(`Activity not read: ${String(e)}`);
    } finally {
      setLoading(false);
    }
  }

  let unlisten: (() => void) | undefined;
  onMount(() => {
    void subscribeActivity((appended) => merge(appended, false)).then((u) => (unlisten = u));
    void load(false);
  });
  onCleanup(() => unlisten?.());

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
    <section class="activity-feed" aria-labelledby="activity-feed-title">
      <div class="activity-feed-title" id="activity-feed-title">
        Activity
      </div>
      <Show when={error()}>
        <div class="form-error" role="alert">
          {error()}
        </div>
      </Show>
      <Show
        when={entries().length > 0}
        fallback={<div class="activity-feed-empty">{loading() ? "Loading…" : "No activity yet."}</div>}
      >
        <ul class="activity-feed-list">
          <For each={entries()}>
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
      <Show when={more()}>
        <button type="button" class="form-btn-secondary activity-feed-more" disabled={loading()} onClick={() => void load(true)}>
          View More
        </button>
      </Show>
    </section>
  );
};

export default ActivityFeed;
