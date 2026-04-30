import { Component, For, Show, createSignal, createMemo } from "solid-js";
import type { Channel, Category } from "../../stores/community.store";
import ContextMenu from "../common/ContextMenu";
import type { ContextMenuItem } from "../common/ContextMenu";
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
} from "../../icons";
import { createContextMenu } from "../../hooks/createContextMenu";

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

  // Channels grouped by category (includes empty categories)
  const categorizedChannels = createMemo(() => {
    const groups: { category: Category | null; channels: Channel[] }[] = [];

    // First: uncategorized channels (no categoryId)
    const uncategorized = props.channels.filter((ch) => !ch.categoryId);
    if (uncategorized.length > 0) {
      groups.push({ category: null, channels: uncategorized });
    }

    // Then: each category in sort order (even if empty)
    for (const cat of sortedCategories()) {
      const catChannels = props.channels.filter((ch) => ch.categoryId === cat.id);
      groups.push({ category: cat, channels: catChannels });
    }

    return groups;
  });

  function channelIcon(ch: Channel): string {
    if (ch.type === "voice") return ICON_VOLUME_HIGH;
    if (ch.type === "announcement") return ICON_MEGAPHONE;
    return ICON_CHANNEL_TEXT;
  }

  function channelIconClass(ch: Channel): string {
    if (ch.type === "announcement") return "nf-icon channel-icon announcement-icon";
    return "nf-icon channel-icon";
  }

  const menu = createContextMenu<Channel>();
  const catMenu = createContextMenu<Category>();

  function handleContextMenu(e: MouseEvent, channel: Channel): void {
    menu.open(e, channel);
  }

  function handleCategoryContextMenu(e: MouseEvent, category: Category): void {
    if (!props.canManage) return;
    catMenu.open(e, category);
  }

  function contextMenuItems(): ContextMenuItem[] {
    const ctx = menu.state();
    if (!ctx) return [];
    const items: ContextMenuItem[] = [];

    if (props.canManage) {
      items.push({
        label: "Rename Channel",
        icon: ICON_PENCIL,
        action: () => {
          props.onRename?.(ctx.data.id, ctx.data.name);
        },
      });
      items.push({
        label: "Delete Channel",
        icon: ICON_DELETE,
        action: () => {
          props.onDelete?.(ctx.data.id);
        },
        danger: true,
      });
    }

    if (props.onSetNotification) {
      items.push({
        label: "Notify: All",
        icon: ICON_BELL,
        action: () => props.onSetNotification!(ctx.data.id, "all"),
      });
      items.push({
        label: "Notify: Mentions",
        icon: ICON_BELL,
        action: () => props.onSetNotification!(ctx.data.id, "mentions"),
      });
      items.push({
        label: "Notify: Nothing",
        icon: ICON_BELL,
        action: () => props.onSetNotification!(ctx.data.id, "nothing"),
      });
    }

    return items;
  }

  function categoryContextMenuItems(): ContextMenuItem[] {
    const ctx = catMenu.state();
    if (!ctx) return [];
    return [
      {
        label: "Rename Category",
        icon: ICON_PENCIL,
        action: () => {
          props.onRenameCategory?.(ctx.data.id, ctx.data.name);
        },
      },
      {
        label: "Delete Category",
        icon: ICON_DELETE,
        action: () => {
          props.onDeleteCategory?.(ctx.data.id);
        },
        danger: true,
      },
    ];
  }

  function renderChannel(channel: Channel) {
    return (
      <div
        class={`channel-item ${props.selectedId === channel.id ? "channel-item-selected" : ""}`}
        onClick={() => props.onSelect(channel.id)}
        onContextMenu={(e) => handleContextMenu(e, channel)}
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
            onClick={(e) => {
              e.stopPropagation();
              props.onVoiceJoin?.(channel.id);
            }}
          >
            <span class="nf-icon">{ICON_PHONE}</span>
          </button>
        )}
      </div>
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
                  <div onContextMenu={(e) => handleCategoryContextMenu(e, cat())}>
                    <CategoryHeader
                      name={cat().name}
                      isExpanded={!isCollapsed()}
                      onToggle={() => toggleCategory(cat().id)}
                    />
                  </div>
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
          <span class="nf-icon">{ICON_PLUS_BOX}</span> Category
        </button>
      </Show>

      <Show when={menu.state()}>
        {(pos) => (
          <ContextMenu
            items={contextMenuItems()}
            x={pos().x}
            y={pos().y}
            onClose={menu.close}
          />
        )}
      </Show>
      <Show when={catMenu.state()}>
        {(pos) => (
          <ContextMenu
            items={categoryContextMenuItems()}
            x={pos().x}
            y={pos().y}
            onClose={catMenu.close}
          />
        )}
      </Show>
    </div>
  );
};

export default ChannelList;
