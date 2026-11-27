import { Component, For, Show, createSignal, createMemo, JSX } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";
import { ContextMenu } from "@kobalte/core/context-menu";
import type { Channel, Category, Member } from "../../stores/community.store";
import { voiceState } from "../../stores/voice.store";
import CategoryHeader from "./CategoryHeader";
import { rovingList } from "../../utils/roving_list";
import type { GlyphName } from "../common/glyph_art";

interface ChannelListProps {
  channels: Channel[];
  categories: Category[];
  members: Member[];
  /** Our own pseudonym in this community, as the Host serves it: the presence badge counts the others. */
  myPseudonymKey: string | null;
  selectedId?: string;
  communityId: string;
  /** Why we may not manage channels, in the Host's words, or null (never-hide). */
  manageDenied: string | null;
  onSelect: (id: string) => void;
  onVoiceJoin?: (id: string) => void;
  onRename?: (channelId: string, currentName: string) => void;
  onDelete?: (channelId: string) => void;
  onRenameCategory?: (categoryId: string, currentName: string) => void;
  onDeleteCategory?: (categoryId: string) => void;
  onCreateCategory?: () => void;
  onSetNotification?: (channelId: string, level: "all" | "mentions" | "nothing") => void;
}

/** The presence badge's meaning in words, so its number never reads as unread. */
function membersHere(n: number): string {
  return `${n} ${n === 1 ? "member" : "members"} here`;
}

const ChannelList: Component<ChannelListProps> = (props) => {
  const [collapsedCategories, setCollapsedCategories] = createSignal<Set<string>>(new Set());

  function toggleCategory(catId: string): void {
    setCollapsedCategories((prev) => {
      const next = new Set(prev);
      if (next.has(catId)) next.delete(catId);
      else next.add(catId);
      return next;
    });
  }

  const sortedCategories = createMemo(() =>
    [...props.categories].sort((a, b) => a.sortOrder - b.sortOrder),
  );

  // Roster grouping: count the other members focused on each channel. Drives
  // the "● N" presence indicator. Only members who share their location have
  // a non-null `location` (default-deny), so this naturally reflects
  // exactly what peers consented to expose. We are not counted: "1 here"
  // with only ourselves in the channel says nothing.
  const presenceByChannel = createMemo(() => {
    const counts: Record<string, number> = {};
    for (const m of props.members) {
      if (m.pseudonymKey === props.myPseudonymKey) continue;
      const loc = m.location;
      if (loc) counts[loc.channelId] = (counts[loc.channelId] ?? 0) + 1;
    }
    return counts;
  });

  // Architecture §10.8 — text-in-voice channels are only visible while the
  // viewer is connected to the parent voice channel.
  const visibleChannels = createMemo(() =>
    props.channels.filter((ch) =>
      !ch.parentVoiceChannelId || ch.parentVoiceChannelId === voiceState.channelId,
    ),
  );

  const categorizedChannels = createMemo(() => {
    const groups: { category: Category | null; channels: Channel[] }[] = [];
    const uncategorized = visibleChannels().filter((ch) => !ch.categoryId);
    if (uncategorized.length > 0) {
      groups.push({ category: null, channels: uncategorized });
    }
    for (const cat of sortedCategories()) {
      const catChannels = visibleChannels().filter((ch) => ch.categoryId === cat.id);
      groups.push({ category: cat, channels: catChannels });
    }
    return groups;
  });

  // A channel's icon by its kind (plan SK17-7a): a voice channel and a text
  // channel are the Community icons a pack may serve.
  function channelIcon(ch: Channel): GlyphName {
    if (ch.type === "voice") return "voice.server";
    if (ch.type === "stage") return "phone";
    if (ch.type === "announcement") return "megaphone";
    if (ch.type === "forum") return "thread";
    return "chat";
  }

  function channelIconClass(ch: Channel): string {
    if (ch.type === "announcement") return "channel-icon announcement-icon";
    return "channel-icon";
  }

  function channelMenu(channel: Channel): JSX.Element {
    return (
      <ContextMenu.Portal>
        <ContextMenu.Content class="context-menu">
          {/* Never-hide: listed always, disabled with the Host's reason. */}
          <ContextMenu.Item
            class="context-menu-item"
            disabled={props.manageDenied !== null}
            title={props.manageDenied ?? undefined}
            onSelect={() => props.onRename?.(channel.id, channel.name)}
          >
            <SkinGlyph name="pencil" class="context-menu-icon" />
            Rename Channel
          </ContextMenu.Item>
          <ContextMenu.Item
            class="context-menu-item context-menu-item-danger"
            disabled={props.manageDenied !== null}
            title={props.manageDenied ?? undefined}
            onSelect={() => props.onDelete?.(channel.id)}
          >
            <SkinGlyph name="delete" class="context-menu-icon" />
            Delete Channel
          </ContextMenu.Item>
          <Show when={props.onSetNotification}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() => props.onSetNotification!(channel.id, "all")}
            >
              <SkinGlyph name="bell" class="context-menu-icon" />
              Notify: All
            </ContextMenu.Item>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() => props.onSetNotification!(channel.id, "mentions")}
            >
              <SkinGlyph name="bell" class="context-menu-icon" />
              Notify: Mentions
            </ContextMenu.Item>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() => props.onSetNotification!(channel.id, "nothing")}
            >
              <SkinGlyph name="bell" class="context-menu-icon" />
              Notify: Nothing
            </ContextMenu.Item>
          </Show>
        </ContextMenu.Content>
      </ContextMenu.Portal>
    );
  }

  function renderChannel(channel: Channel): JSX.Element {
    return (
      <ContextMenu>
        <ContextMenu.Trigger
          as="div"
          class={`channel-item ${props.selectedId === channel.id ? "channel-item-selected" : ""}`}
          onClick={() => props.onSelect(channel.id)}
          onKeyDown={(e: KeyboardEvent) => {
            if (e.key === "Enter" && e.target === e.currentTarget) props.onSelect(channel.id);
          }}
        >
          <SkinGlyph name={channelIcon(channel)} class={channelIconClass(channel)} />
          <span class="channel-name">{channel.name}</span>
          <Show when={channel.locked}>
            <span class="channel-locked" title={channel.lockedReason} aria-label={channel.lockedReason ? `Locked: ${channel.lockedReason}` : "Locked"}>
              <SkinGlyph name="lock" class="channel-locked-icon" />
            </span>
          </Show>
          {(presenceByChannel()[channel.id] ?? 0) > 0 && (
            <span
              class="channel-presence-badge"
              title={membersHere(presenceByChannel()[channel.id])}
              aria-label={membersHere(presenceByChannel()[channel.id])}
            >
              <span class="channel-presence-dot" aria-hidden="true">●</span>
              {presenceByChannel()[channel.id]}
            </span>
          )}
          {channel.unreadCount > 0 && (
            <span class="channel-unread-badge">{channel.unreadCount}</span>
          )}
          {channel.type === "voice" && (
            <button
              class="channel-voice-join-btn"
              title="Join Voice"
              aria-label={`Join voice channel ${channel.name}`}
              onClick={(e) => {
                e.stopPropagation();
                props.onVoiceJoin?.(channel.id);
              }}
            >
              <SkinGlyph name="phone" />
            </button>
          )}
        </ContextMenu.Trigger>
        {channelMenu(channel)}
      </ContextMenu>
    );
  }

  return (
    <div class="channel-list" ref={rovingList(".channel-item")}>
      <For each={categorizedChannels()}>
        {(group) => {
          const isCollapsed = () => group.category ? collapsedCategories().has(group.category.id) : false;

          return (
            <>
              <Show when={group.category} fallback={
                <div class="channel-section-header">Channels</div>
              }>
                {(cat) => (
                  <ContextMenu>
                    {/* Never-hide: the category's menu is always there; its items are
                        disabled with the Host's reason when we may not manage channels. */}
                    <ContextMenu.Trigger as="div">
                      <CategoryHeader
                        name={cat().name}
                        isExpanded={!isCollapsed()}
                        onToggle={() => toggleCategory(cat().id)}
                      />
                    </ContextMenu.Trigger>
                    <ContextMenu.Portal>
                      <ContextMenu.Content class="context-menu">
                        <ContextMenu.Item
                          class="context-menu-item"
                          disabled={props.manageDenied !== null}
                          title={props.manageDenied ?? undefined}
                          onSelect={() => props.onRenameCategory?.(cat().id, cat().name)}
                        >
                          <SkinGlyph name="pencil" class="context-menu-icon" />
                          Rename Category
                        </ContextMenu.Item>
                        <ContextMenu.Item
                          class="context-menu-item context-menu-item-danger"
                          disabled={props.manageDenied !== null}
                          title={props.manageDenied ?? undefined}
                          onSelect={() => props.onDeleteCategory?.(cat().id)}
                        >
                          <SkinGlyph name="delete" class="context-menu-icon" />
                          Delete Category
                        </ContextMenu.Item>
                      </ContextMenu.Content>
                    </ContextMenu.Portal>
                  </ContextMenu>
                )}
              </Show>
              <Show when={!isCollapsed()}>
                <For each={group.channels}>
                  {(channel) => renderChannel(channel)}
                </For>
              </Show>
            </>
          );
        }}
      </For>

      {/* Never-hide: drawn always, disabled with the Host's reason. */}
      <Show when={props.onCreateCategory}>
        <button
          class="channel-create-category-btn"
          onClick={() => props.onCreateCategory?.()}
          disabled={props.manageDenied !== null}
          title={props.manageDenied ?? "Create Category"}
          aria-label="Create Category"
          aria-description={props.manageDenied ?? undefined}
        >
          <SkinGlyph name="plus-box" /> Category
        </button>
      </Show>
    </div>
  );
};

export default ChannelList;
