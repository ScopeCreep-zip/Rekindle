import { type Component, createMemo, createSignal, For, Show } from "solid-js";
import type { Channel, Thread } from "../../stores/community.store";
import { formatRelativeTime, messageCount } from "../../utils/formatting";
import SkinGlyph from "../common/SkinGlyph";

interface ForumChannelViewProps {
  channel: Channel;
  threads: Thread[];
  onOpenThread: (thread: Thread) => void;
  /** A pseudonym's display name (the window's member resolver). */
  nameOf: (pseudonymKey: string) => string;
  /** Whether the Host created the post: the draft is cleared only then (plan FT1). */
  onCreatePost: (name: string, body: string, forumTag?: string | null) => Promise<boolean>;
}

const ForumChannelView: Component<ForumChannelViewProps> = (props) => {
  const [activeTag, setActiveTag] = createSignal<string | null>(null);
  const [composerOpen, setComposerOpen] = createSignal(false);
  // A forum post is a new thread and its starter, so it needs both thread
  // capabilities (plan FT1d); each lacking one carries the Host's reason.
  // The controls stay shown, disabled with the reasons (owner: never hidden).
  // A channel we may not read (the Host's lock, e.g. a view overwrite) is
  // shown locked with the Host's words, never as an empty forum.
  const lockReason = (): string | null =>
    props.channel.locked ? (props.channel.lockedReason ?? "") : null;
  const postDenied = (): string | null => {
    const lock = lockReason();
    if (lock !== null) return lock;
    const denied = props.channel.denied ?? {};
    const reasons = [denied.createPublicThreads, denied.sendInThreads].filter(
      (r): r is string => !!r
    );
    return reasons.length > 0 ? reasons.join(" ") : null;
  };
  const [title, setTitle] = createSignal("");
  const [body, setBody] = createSignal("");
  const [selectedTag, setSelectedTag] = createSignal<string>("");
  const [isSubmitting, setIsSubmitting] = createSignal(false);

  const sortedThreads = createMemo(() =>
    [...props.threads].sort((a, b) => b.lastMessageAt - a.lastMessageAt)
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
      // A refusal keeps the whole draft open for another try.
      if (!(await props.onCreatePost(nextTitle, nextBody, selectedTag() || null))) return;
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
          <SkinGlyph name="thread" />
          <div class="forum-channel-title-group">
            <div class="forum-channel-title">{props.channel.name}</div>
            <Show when={props.channel.topic}>
              <div class="forum-channel-topic">{props.channel.topic}</div>
            </Show>
          </div>
        </div>
        <button
          type="button"
          class="forum-channel-new-post-btn"
          onClick={() => setComposerOpen(true)}
          disabled={postDenied() !== null}
          title={postDenied() ?? undefined}
          aria-description={postDenied() ?? undefined}
        >
          <SkinGlyph name="plus" />
          New post
        </button>
      </div>

      <Show when={(props.channel.forumTags?.length ?? 0) > 0}>
        <div class="forum-channel-tags">
          <button
            type="button"
            class={`forum-channel-tag ${activeTag() === null ? "forum-channel-tag-active" : ""}`}
            onClick={() => setActiveTag(null)}
          >
            All
          </button>
          <For each={props.channel.forumTags ?? []}>
            {(tag) => (
              <button
                type="button"
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
            <button
              type="button"
              class="forum-channel-composer-close"
              onClick={() => setComposerOpen(false)}
            >
              <SkinGlyph name="close" />
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
            <button type="button" class="btn btn-secondary" onClick={() => setComposerOpen(false)}>
              Cancel
            </button>
            <button
              type="button"
              class="btn btn-primary"
              onClick={() => void handleSubmit()}
              disabled={isSubmitting() || postDenied() !== null}
              title={postDenied() ?? undefined}
              aria-description={postDenied() ?? undefined}
            >
              {isSubmitting() ? "Posting..." : "Create post"}
            </button>
          </div>
        </div>
      </Show>

      <div class="forum-channel-thread-list">
        <Show
          when={visibleThreads().length > 0}
          fallback={
            <Show
              when={lockReason() !== null}
              fallback={
                <div class="forum-channel-empty">
                  <div class="forum-channel-empty-title">No posts yet</div>
                  {/* Where the Host denies posting, its reason, never the invitation. */}
                  <div class="forum-channel-empty-subtitle">
                    {postDenied() ?? "Start the first thread for this forum channel."}
                  </div>
                </div>
              }
            >
              <div class="forum-channel-empty forum-channel-locked" role="status">
                <SkinGlyph name="lock" />
                <div class="forum-channel-empty-title">This forum is locked</div>
                <div class="forum-channel-empty-subtitle">{lockReason()}</div>
              </div>
            </Show>
          }
        >
          <For each={visibleThreads()}>
            {(thread) => (
              <button
                type="button"
                class="forum-channel-thread-card"
                onClick={() => props.onOpenThread(thread)}
              >
                <div class="forum-channel-thread-topline">
                  <div class="forum-channel-thread-name">{thread.name}</div>
                  <Show when={thread.forumTag}>
                    <span class="forum-channel-thread-tag">{thread.forumTag}</span>
                  </Show>
                </div>
                <div class="forum-channel-thread-meta">
                  Started by {props.nameOf(thread.creatorPseudonym)} ·{" "}
                  {messageCount(thread.messageCount)} ·{" "}
                  {formatRelativeTime(thread.lastMessageAt * 1000)}
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
