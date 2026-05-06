import { Component, For, Show, createSignal, createMemo, JSX } from "solid-js";
import { ContextMenu } from "@kobalte/core/context-menu";
import type { Channel, Category } from "../../stores/community.store";
import { voiceState } from "../../stores/voice.store";
import CategoryHeader from "./CategoryHeader";
import {
  ICON_CHANNEL_TEXT,
  ICON_VOLUME_HIGH,
  ICON_MEGAPHONE,
  ICON_PHONE,
  ICON_PENCIL,
  ICON_DELETE,
  ICON_PLUS_BOX,
  ICON_BELL,
  ICON_THREAD,
} from "../../icons";

interface ChannelListProps {
  channels: Channel[];
  categories: Category[];
  selectedId?: string;
  communityId: string;
  canManage: boolean;
  onSelect: (id: string) => void;
  onVoiceJoin?: (id: string) => void;
  onRename?: (channelId: string, currentName: string) => void;
  onDelete?: (channelId: string) => void;
  onRenameCategory?: (categoryId: string, currentName: string) => void;
  onDeleteCategory?: (categoryId: string) => void;
  onCreateCategory?: () => void;
  onSetNotification?: (channelId: string, level: "all" | "mentions" | "nothing") => void;
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

  function channelIcon(ch: Channel): string {
    if (ch.type === "voice") return ICON_VOLUME_HIGH;
    if (ch.type === "stage") return ICON_PHONE;
    if (ch.type === "announcement") return ICON_MEGAPHONE;
    if (ch.type === "forum") return ICON_THREAD;
    return ICON_CHANNEL_TEXT;
  }

  function channelIconClass(ch: Channel): string {
    if (ch.type === "announcement") return "nf-icon channel-icon announcement-icon";
    return "nf-icon channel-icon";
  }

  function channelMenu(channel: Channel): JSX.Element {
    return (
      <ContextMenu.Portal>
        <ContextMenu.Content class="context-menu">
          <Show when={props.canManage}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() => props.onRename?.(channel.id, channel.name)}
            >
              <span class="nf-icon context-menu-icon">{ICON_PENCIL}</span>
              Rename Channel
            </ContextMenu.Item>
            <ContextMenu.Item
              class="context-menu-item context-menu-item-danger"
              onSelect={() => props.onDelete?.(channel.id)}
            >
              <span class="nf-icon context-menu-icon">{ICON_DELETE}</span>
              Delete Channel
            </ContextMenu.Item>
          </Show>
          <Show when={props.onSetNotification}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() => props.onSetNotification!(channel.id, "all")}
            >
              <span class="nf-icon context-menu-icon">{ICON_BELL}</span>
              Notify: All
            </ContextMenu.Item>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() => props.onSetNotification!(channel.id, "mentions")}
            >
              <span class="nf-icon context-menu-icon">{ICON_BELL}</span>
              Notify: Mentions
            </ContextMenu.Item>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() => props.onSetNotification!(channel.id, "nothing")}
            >
              <span class="nf-icon context-menu-icon">{ICON_BELL}</span>
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
        >
          <span class={channelIconClass(channel)}>{channelIcon(channel)}</span>
          <span class="channel-name">{channel.name}</span>
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
              <span class="nf-icon" aria-hidden="true">{ICON_PHONE}</span>
            </button>
          )}
        </ContextMenu.Trigger>
        {channelMenu(channel)}
      </ContextMenu>
    );
  }

  return (
    <div class="channel-list">
      <For each={categorizedChannels()}>
        {(group) => {
          const isCollapsed = () => group.category ? collapsedCategories().has(group.category.id) : false;

          return (
            <>
              <Show when={group.category} fallback={
                <div class="channel-section-header">Channels</div>
              }>
                {(cat) => (
                  <Show
                    when={props.canManage}
                    fallback={
                      <CategoryHeader
                        name={cat().name}
                        isExpanded={!isCollapsed()}
                        onToggle={() => toggleCategory(cat().id)}
                      />
                    }
                  >
                    <ContextMenu>
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
                            onSelect={() => props.onRenameCategory?.(cat().id, cat().name)}
                          >
                            <span class="nf-icon context-menu-icon">{ICON_PENCIL}</span>
                            Rename Category
                          </ContextMenu.Item>
                          <ContextMenu.Item
                            class="context-menu-item context-menu-item-danger"
                            onSelect={() => props.onDeleteCategory?.(cat().id)}
                          >
                            <span class="nf-icon context-menu-icon">{ICON_DELETE}</span>
                            Delete Category
                          </ContextMenu.Item>
                        </ContextMenu.Content>
                      </ContextMenu.Portal>
                    </ContextMenu>
                  </Show>
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

      <Show when={props.canManage && props.onCreateCategory}>
        <button
          class="channel-create-category-btn"
          onClick={() => props.onCreateCategory?.()}
          title="Create Category"
        >
          <span class="nf-icon" aria-hidden="true">{ICON_PLUS_BOX}</span> Category
        </button>
      </Show>
    </div>
  );
};

export default ChannelList;
