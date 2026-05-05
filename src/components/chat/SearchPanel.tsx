import { Component, For, Show, createMemo, createSignal, onMount } from "solid-js";
import {
  commands,
  type SearchHit,
  type SearchResult,
  type SearchSort,
  type HasFilter,
} from "../../ipc/commands";
import { communityState } from "../../stores/community.store";
import { formatTimestamp } from "../../utils/formatting";

// Architecture §23 — local FTS5 search across messages, threads, DMs.
// Body, sender, channel, thread, has-filter, date range, mentions and
// pinned-only constraints are all backed by the same Tauri command.

interface SearchPanelProps {
  /** Default community to scope to. `null` = global. */
  communityId?: string | null;
  /** Default channel to scope to when scope is "channel". */
  channelId?: string | null;
  onClose: () => void;
}

type SearchScopeMode = "channel" | "community" | "global";

const HAS_OPTIONS: { value: HasFilter; label: string }[] = [
  { value: "link", label: "Link" },
  { value: "file", label: "File" },
  { value: "image", label: "Image" },
  { value: "video", label: "Video" },
  { value: "embed", label: "Embed" },
  { value: "poll", label: "Poll" },
  { value: "voice_message", label: "Voice message" },
];

const SearchPanel: Component<SearchPanelProps> = (props) => {
  let queryRef: HTMLInputElement | undefined;
  const [query, setQuery] = createSignal("");
  const [from, setFrom] = createSignal("");
  // Architecture §32 Phase 7 W23 — three-tier scope: this channel /
  // this community / all communities. The active scope determines
  // which `inChannel` + `inCommunity` filters travel to the backend.
  const initialScope: SearchScopeMode = props.channelId
    ? "channel"
    : props.communityId
      ? "community"
      : "global";
  const [scope, setScope] = createSignal<SearchScopeMode>(initialScope);
  const [inChannel, setInChannel] = createSignal(props.channelId ?? "");
  const [hasFilters, setHasFilters] = createSignal<HasFilter[]>([]);
  const [before, setBefore] = createSignal("");
  const [after, setAfter] = createSignal("");
  const [pinnedOnly, setPinnedOnly] = createSignal(false);
  const [sort, setSort] = createSignal<SearchSort>("relevance");
  const [hits, setHits] = createSignal<SearchHit[]>([]);
  const [searching, setSearching] = createSignal(false);
  const [queryMs, setQueryMs] = createSignal<number | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  onMount(() => queryRef?.focus());

  const community = createMemo(() => {
    if (!props.communityId) return undefined;
    return communityState.communities[props.communityId];
  });

  const channels = createMemo(() => community()?.channels ?? []);
  const members = createMemo(() => community()?.members ?? []);

  function toggleHas(value: HasFilter): void {
    setHasFilters((prev) =>
      prev.includes(value) ? prev.filter((v) => v !== value) : [...prev, value],
    );
  }

  function parseDate(raw: string): number | undefined {
    if (!raw) return undefined;
    const ts = Date.parse(raw);
    return Number.isNaN(ts) ? undefined : Math.floor(ts / 1000);
  }

  async function runSearch(): Promise<void> {
    const q = query().trim();
    if (!q) {
      setHits([]);
      setQueryMs(null);
      return;
    }
    setSearching(true);
    setError(null);
    try {
      const activeScope = scope();
      const inCommunityFilter =
        activeScope === "global" ? undefined : props.communityId ?? undefined;
      const inChannelFilter =
        activeScope === "channel" ? (inChannel() || props.channelId) ?? undefined : undefined;
      const result: SearchResult = await commands.searchMessages({
        query: q,
        filters: {
          from: from() || undefined,
          inCommunity: inCommunityFilter,
          inChannel: inChannelFilter,
          has: hasFilters().length > 0 ? hasFilters() : undefined,
          before: parseDate(before()),
          after: parseDate(after()),
          isPinned: pinnedOnly() ? true : undefined,
        },
        sort: sort(),
        limit: 50,
      });
      setHits(result.hits);
      setQueryMs(result.queryMs);
    } catch (e) {
      console.error("Search failed:", e);
      setError(typeof e === "string" ? e : "Search failed");
    } finally {
      setSearching(false);
    }
  }

  function onKeyDown(e: KeyboardEvent): void {
    if (e.key === "Escape") {
      e.preventDefault();
      props.onClose();
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      void runSearch();
    }
  }

  return (
    <div class="search-panel-overlay" onClick={props.onClose}>
      <div class="search-panel" onClick={(e) => e.stopPropagation()}>
        <div class="search-panel-header">
          <h3>Search messages</h3>
          <button
            class="search-panel-close"
            onClick={props.onClose}
            title="Close (Esc)"
            aria-label="Close search panel"
          >×</button>
        </div>

        <div class="search-panel-input-row">
          <input
            ref={queryRef}
            class="form-input search-panel-query"
            type="text"
            placeholder="Search…"
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            onKeyDown={onKeyDown}
          />
          <select
            class="form-select"
            value={scope()}
            onChange={(e) => setScope(e.currentTarget.value as SearchScopeMode)}
            aria-label="Search scope"
          >
            <option value="channel" disabled={!props.channelId}>This channel</option>
            <option value="community" disabled={!props.communityId}>This community</option>
            <option value="global">All communities</option>
          </select>
          <select
            class="form-select"
            value={sort()}
            onChange={(e) => setSort(e.currentTarget.value as SearchSort)}
            aria-label="Sort order"
          >
            <option value="relevance">Relevance</option>
            <option value="newest">Newest</option>
            <option value="oldest">Oldest</option>
          </select>
          <button
            class="form-btn-primary"
            onClick={() => void runSearch()}
            disabled={searching() || query().trim().length === 0}
          >
            {searching() ? "…" : "Search"}
          </button>
        </div>
        <div class="settings-hint">
          FTS5 query syntax — phrase, AND, OR, prefix*. Regex is not
          supported. Searches are local-only; only messages stored on
          this device can match.
        </div>

        <div class="search-panel-filters">
          <Show when={community()}>
            <select
              class="form-select"
              value={from()}
              onChange={(e) => setFrom(e.currentTarget.value)}
            >
              <option value="">From: anyone</option>
              <For each={members()}>
                {(member) => (
                  <option value={member.pseudonymKey}>From: {member.displayName}</option>
                )}
              </For>
            </select>
            <select
              class="form-select"
              value={inChannel()}
              onChange={(e) => setInChannel(e.currentTarget.value)}
            >
              <option value="">In: any channel</option>
              <For each={channels()}>
                {(channel) => (
                  <option value={channel.id}>In: #{channel.name}</option>
                )}
              </For>
            </select>
          </Show>
          <input
            class="form-input"
            type="date"
            value={after()}
            onInput={(e) => setAfter(e.currentTarget.value)}
            title="After date"
          />
          <input
            class="form-input"
            type="date"
            value={before()}
            onInput={(e) => setBefore(e.currentTarget.value)}
            title="Before date"
          />
          <label class="search-panel-pinned">
            <input
              type="checkbox"
              checked={pinnedOnly()}
              onChange={(e) => setPinnedOnly(e.currentTarget.checked)}
            />
            <span>Pinned only</span>
          </label>
        </div>

        <div class="search-panel-has">
          <For each={HAS_OPTIONS}>
            {(opt) => (
              <button
                class={`search-panel-chip ${hasFilters().includes(opt.value) ? "search-panel-chip-active" : ""}`}
                onClick={() => toggleHas(opt.value)}
              >
                {opt.label}
              </button>
            )}
          </For>
        </div>

        <Show when={error()}>
          <div class="search-panel-error">{error()}</div>
        </Show>

        <Show when={queryMs() !== null && hits().length === 0 && !searching()}>
          <div class="search-panel-empty">No results.</div>
        </Show>

        <div class="search-panel-results">
          <For each={hits()}>
            {(hit) => (
              <div class="search-panel-hit">
                <div class="search-panel-hit-meta">
                  <span class={`search-panel-hit-scope search-panel-hit-scope-${hit.scope}`}>
                    {hit.scope}
                  </span>
                  <span class="search-panel-hit-time">{formatTimestamp(hit.timestamp)}</span>
                </div>
                <div class="search-panel-hit-body">
                  <Show when={hit.beforeBody}>
                    <div class="search-panel-hit-context">{hit.beforeBody}</div>
                  </Show>
                  <div class="search-panel-hit-match">{hit.body}</div>
                  <Show when={hit.afterBody}>
                    <div class="search-panel-hit-context">{hit.afterBody}</div>
                  </Show>
                </div>
              </div>
            )}
          </For>
        </div>

        <Show when={queryMs() !== null}>
          <div class="search-panel-footer">
            {hits().length} result{hits().length === 1 ? "" : "s"} · {queryMs()} ms
          </div>
        </Show>
      </div>
    </div>
  );
};

export default SearchPanel;
