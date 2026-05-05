import { Component, For, Show, createMemo, createSignal } from "solid-js";
import type { Channel, Thread } from "../../stores/community.store";
import { formatRelativeTime } from "../../utils/formatting";
import { ICON_PLUS, ICON_THREAD, ICON_CLOSE } from "../../icons";

interface ForumChannelViewProps {
  channel: Channel;
  threads: Thread[];
  onOpenThread: (thread: Thread) => void;
  onCreatePost: (name: string, body: string, forumTag?: string | null) => Promise<void> | void;
}

const ForumChannelView: Component<ForumChannelViewProps> = (props) => {
  const [activeTag, setActiveTag] = createSignal<string | null>(null);
  const [composerOpen, setComposerOpen] = createSignal(false);
  const [title, setTitle] = createSignal("");
  const [body, setBody] = createSignal("");
  const [selectedTag, setSelectedTag] = createSignal<string>("");
  const [isSubmitting, setIsSubmitting] = createSignal(false);

  const sortedThreads = createMemo(() =>
    [...props.threads].sort((a, b) => b.lastMessageAt - a.lastMessageAt),
  );

  const visibleThreads = createMemo(() => {
    const tag = activeTag();
    return sortedThreads().filter((thread) => {
      if (!tag) return true;
      return thread.forumTag === tag;
    });
  });

  async function handleSubmit(): Promise<void> {
    const nextTitle = title().trim();
    const nextBody = body().trim();
    if (!nextTitle || !nextBody || isSubmitting()) return;
    setIsSubmitting(true);
    try {
      await props.onCreatePost(nextTitle, nextBody, selectedTag() || null);
      setTitle("");
      setBody("");
      setSelectedTag("");
      setComposerOpen(false);
    } finally {
      setIsSubmitting(false);
    }
  }

  return (
    <div class="forum-channel-view">
      <div class="forum-channel-toolbar">
        <div class="forum-channel-heading">
          <span class="nf-icon">{ICON_THREAD}</span>
          <div class="forum-channel-title-group">
            <div class="forum-channel-title">{props.channel.name}</div>
            <Show when={props.channel.topic}>
              <div class="forum-channel-topic">{props.channel.topic}</div>
            </Show>
          </div>
        </div>
        <button class="forum-channel-new-post-btn" onClick={() => setComposerOpen(true)}>
          <span class="nf-icon">{ICON_PLUS}</span>
          New post
        </button>
      </div>

      <Show when={(props.channel.forumTags?.length ?? 0) > 0}>
        <div class="forum-channel-tags">
          <button
            class={`forum-channel-tag ${activeTag() === null ? "forum-channel-tag-active" : ""}`}
            onClick={() => setActiveTag(null)}
          >
            All
          </button>
          <For each={props.channel.forumTags ?? []}>
            {(tag) => (
              <button
                class={`forum-channel-tag ${activeTag() === tag ? "forum-channel-tag-active" : ""}`}
                onClick={() => setActiveTag(tag)}
              >
                {tag}
              </button>
            )}
          </For>
        </div>
      </Show>

      <Show when={composerOpen()}>
        <div class="forum-channel-composer">
          <div class="forum-channel-composer-header">
            <div class="forum-channel-composer-title">Create forum post</div>
            <button class="forum-channel-composer-close" onClick={() => setComposerOpen(false)}>
              <span class="nf-icon">{ICON_CLOSE}</span>
            </button>
          </div>
          <input
            class="form-input forum-channel-input"
            type="text"
            placeholder="Post title"
            value={title()}
            onInput={(e) => setTitle(e.currentTarget.value)}
          />
          <Show when={(props.channel.forumTags?.length ?? 0) > 0}>
            <select
              class="form-input forum-channel-select"
              value={selectedTag()}
              onChange={(e) => setSelectedTag(e.currentTarget.value)}
            >
              <option value="">No tag</option>
              <For each={props.channel.forumTags ?? []}>
                {(tag) => <option value={tag}>{tag}</option>}
              </For>
            </select>
          </Show>
          <textarea
            class="message-input message-input-field forum-channel-body"
            rows={5}
            placeholder="Write the opening post..."
            value={body()}
            onInput={(e) => setBody(e.currentTarget.value)}
          />
          <div class="forum-channel-composer-actions">
            <button class="btn btn-secondary" onClick={() => setComposerOpen(false)}>
              Cancel
            </button>
            <button class="btn btn-primary" onClick={() => void handleSubmit()} disabled={isSubmitting()}>
              {isSubmitting() ? "Posting..." : "Create post"}
            </button>
          </div>
        </div>
      </Show>

      <div class="forum-channel-thread-list">
        <Show when={visibleThreads().length > 0} fallback={
          <div class="forum-channel-empty">
            <div class="forum-channel-empty-title">No posts yet</div>
            <div class="forum-channel-empty-subtitle">Start the first thread for this forum channel.</div>
          </div>
        }>
          <For each={visibleThreads()}>
            {(thread) => (
              <button class="forum-channel-thread-card" onClick={() => props.onOpenThread(thread)}>
                <div class="forum-channel-thread-topline">
                  <div class="forum-channel-thread-name">{thread.name}</div>
                  <Show when={thread.forumTag}>
                    <span class="forum-channel-thread-tag">{thread.forumTag}</span>
                  </Show>
                </div>
                <div class="forum-channel-thread-meta">
                  Started by {thread.creatorPseudonym} · {thread.messageCount} messages · {formatRelativeTime(thread.lastMessageAt * 1000)}
                </div>
              </button>
            )}
          </For>
        </Show>
      </div>
    </div>
  );
};

export default ForumChannelView;
