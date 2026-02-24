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
  const [showArchived, setShowArchived] = createSignal(false);
  const [sortMode, setSortMode] = createSignal<SortMode>("activity");
  const [searchQuery, setSearchQuery] = createSignal("");

  const filteredActive = createMemo(() => {
    const q = searchQuery().toLowerCase();
    let threads = props.threads.filter((t) => !t.archived);
    if (q) threads = threads.filter((t) => t.name.toLowerCase().includes(q));
    const mode = sortMode();
    if (mode === "activity") {
      return [...threads].sort((a, b) => b.lastMessageAt - a.lastMessageAt);
    }
    return [...threads].sort((a, b) => b.createdAt - a.createdAt);
  });

  const archivedThreads = createMemo(() => {
    const q = searchQuery().toLowerCase();
    let threads = props.threads.filter((t) => t.archived);
    if (q) threads = threads.filter((t) => t.name.toLowerCase().includes(q));
    return threads;
  });

  return (
    <div class="thread-list-panel">
      <div class="thread-panel-header">
        <span class="nf-icon">{ICON_THREAD}</span>
        Threads
        <button class="modal-close-btn" onClick={props.onClose}>
          <span class="nf-icon">{ICON_CLOSE}</span>
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
        <Show when={filteredActive().length === 0 && archivedThreads().length === 0}>
          <div class="pin-panel-empty">No threads yet</div>
        </Show>
        <For each={filteredActive()}>
          {(thread) => (
            <div class="thread-list-item" onClick={() => props.onSelectThread(thread.id)}>
              <div class="thread-list-item-name">{thread.name}</div>
              <div class="thread-list-item-meta">
                {thread.messageCount} messages - {formatRelativeTime(thread.lastMessageAt * 1000)}
              </div>
            </div>
          )}
        </For>
        <Show when={archivedThreads().length > 0}>
          <button class="thread-list-toggle-archived" onClick={() => setShowArchived(!showArchived())}>
            {showArchived() ? "Hide" : "Show"} Archived ({archivedThreads().length})
          </button>
          <Show when={showArchived()}>
            <For each={archivedThreads()}>
              {(thread) => (
                <div class="thread-list-item thread-archived" onClick={() => props.onSelectThread(thread.id)}>
                  <div class="thread-list-item-name">{thread.name}</div>
                  <div class="thread-list-item-meta">
                    {thread.messageCount} messages - archived
                  </div>
                </div>
              )}
            </For>
          </Show>
        </Show>
      </div>
    </div>
  );
};

export default ThreadListPanel;
