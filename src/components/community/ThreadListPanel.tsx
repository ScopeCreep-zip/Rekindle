import { type Component, For, Show, createSignal, createMemo } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";
import type { Thread } from "../../stores/community.store";
import { formatRelativeTime } from "../../utils/formatting";

interface ThreadListPanelProps {
  threads: Thread[];
  onSelectThread: (threadId: string) => void;
  onClose: () => void;
}

type SortMode = "activity" | "created";

const ThreadListPanel: Component<ThreadListPanelProps> = (props) => {
  const [sortMode, setSortMode] = createSignal<SortMode>("activity");
  const [searchQuery, setSearchQuery] = createSignal("");

  const sortedThreads = createMemo(() => {
    const q = searchQuery().toLowerCase();
    let threads = props.threads;
    if (q) threads = threads.filter((t) => t.name.toLowerCase().includes(q));
    const mode = sortMode();
    if (mode === "activity") {
      return [...threads].sort((a, b) => b.lastMessageAt - a.lastMessageAt);
    }
    return [...threads].sort((a, b) => b.createdAt - a.createdAt);
  });

  // Archived threads stay reachable, listed apart (Discord's "Archived
  // threads"; plan E7.13 walk 18 (b)).
  const active = createMemo(() => sortedThreads().filter((t) => !t.archived));
  const archived = createMemo(() => sortedThreads().filter((t) => t.archived));
  const item = (thread: Thread) => (
    <div
      class={`thread-list-item${thread.archived ? " thread-archived" : ""}`}
      onClick={() => props.onSelectThread(thread.id)}
    >
      <div class="thread-list-item-name">{thread.name}</div>
      <div class="thread-list-item-meta">
        {thread.messageCount} messages - {formatRelativeTime(thread.lastMessageAt * 1000)}
      </div>
    </div>
  );

  return (
    <div class="thread-list-panel">
      <div class="thread-panel-header">
        <SkinGlyph name="thread" />
        Threads
        <button
          type="button"
          class="modal-close-btn"
          onClick={props.onClose}
          aria-label="Close threads panel"
        >
          <SkinGlyph name="close" />
        </button>
      </div>
      <div class="thread-list-controls">
        <input
          class="member-search-input"
          type="text"
          placeholder="Search threads..."
          value={searchQuery()}
          onInput={(e) => setSearchQuery(e.currentTarget.value)}
        />
        <div class="thread-list-sort-row">
          <button
            type="button"
            class={`event-filter-btn ${sortMode() === "activity" ? "event-filter-active" : ""}`}
            onClick={() => setSortMode("activity")}
          >
            Recent
          </button>
          <button
            type="button"
            class={`event-filter-btn ${sortMode() === "created" ? "event-filter-active" : ""}`}
            onClick={() => setSortMode("created")}
          >
            Newest
          </button>
        </div>
      </div>
      <div class="thread-list-content">
        <Show when={sortedThreads().length === 0}>
          <div class="pin-panel-empty">No threads yet</div>
        </Show>
        <For each={active()}>{item}</For>
        <Show when={archived().length > 0}>
          <div class="thread-list-section">Archived threads</div>
          <For each={archived()}>{item}</For>
        </Show>
      </div>
    </div>
  );
};

export default ThreadListPanel;
