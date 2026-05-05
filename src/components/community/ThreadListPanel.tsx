import { Component, For, Show, createSignal, createMemo } from "solid-js";
import type { Thread } from "../../stores/community.store";
import { ICON_CLOSE, ICON_THREAD } from "../../icons";
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

  const filteredThreads = createMemo(() => {
    const q = searchQuery().toLowerCase();
    let threads = props.threads;
    if (q) threads = threads.filter((t) => t.name.toLowerCase().includes(q));
    const mode = sortMode();
    if (mode === "activity") {
      return [...threads].sort((a, b) => b.lastMessageAt - a.lastMessageAt);
    }
    return [...threads].sort((a, b) => b.createdAt - a.createdAt);
  });

  return (
    <div class="thread-list-panel">
      <div class="thread-panel-header">
        <span class="nf-icon" aria-hidden="true">{ICON_THREAD}</span>
        Threads
        <button class="modal-close-btn" onClick={props.onClose} aria-label="Close threads panel">
          <span class="nf-icon" aria-hidden="true">{ICON_CLOSE}</span>
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
            class={`event-filter-btn ${sortMode() === "activity" ? "event-filter-active" : ""}`}
            onClick={() => setSortMode("activity")}
          >
            Recent
          </button>
          <button
            class={`event-filter-btn ${sortMode() === "created" ? "event-filter-active" : ""}`}
            onClick={() => setSortMode("created")}
          >
            Newest
          </button>
        </div>
      </div>
      <div class="thread-list-content">
        <Show when={filteredThreads().length === 0}>
          <div class="pin-panel-empty">No threads yet</div>
        </Show>
        <For each={filteredThreads()}>
          {(thread) => (
            <div class="thread-list-item" onClick={() => props.onSelectThread(thread.id)}>
              <div class="thread-list-item-name">{thread.name}</div>
              <div class="thread-list-item-meta">
                {thread.messageCount} messages - {formatRelativeTime(thread.lastMessageAt * 1000)}
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

export default ThreadListPanel;
