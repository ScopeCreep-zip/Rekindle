import { Component, For, Show } from "solid-js";
import { communityState, Community } from "../../stores/community.store";
import ContextMenu from "../common/ContextMenu";
import type { ContextMenuItem } from "../common/ContextMenu";
import { createContextMenu } from "../../hooks/createContextMenu";
import { ICON_SETTINGS, ICON_LOGOUT, ICON_COPY } from "../../icons";

interface CommunityListProps {
  selectedId?: string;
  onSelect: (id: string) => void;
  onSettings?: (id: string) => void;
  onLeave?: (id: string) => void;
}

const CommunityList: Component<CommunityListProps> = (props) => {
  const list = () => Object.values(communityState.communities);
  const menu = createContextMenu<Community>();

  function unreads(c: Community): number {
    return c.channels.reduce((sum, ch) => sum + (ch.unreadCount ?? 0), 0);
  }

  function contextMenuItems(): ContextMenuItem[] {
    const ctx = menu.state();
    if (!ctx) return [];
    const community = ctx.data;
    const items: ContextMenuItem[] = [];

    if (community.isHosted && props.onSettings) {
      items.push({
        label: "Settings",
        icon: ICON_SETTINGS,
        action: () => props.onSettings!(community.id),
      });
    }

    items.push({
      label: "Copy ID",
      icon: ICON_COPY,
      action: () => navigator.clipboard.writeText(community.id),
    });

    if (props.onLeave) {
      items.push({
        label: "Leave",
        icon: ICON_LOGOUT,
        action: () => props.onLeave!(community.id),
        danger: true,
      });
    }

    return items;
  }

  return (
    <div class="community-list">
      <For each={list()} fallback={
        <div class="empty-placeholder">
          <div class="empty-placeholder-subtitle">No communities</div>
        </div>
      }>
        {(community: Community) => (
          <div
            class={`community-item ${props.selectedId === community.id ? "community-item-selected" : ""}`}
            onClick={() => props.onSelect(community.id)}
            onContextMenu={(e) => menu.open(e, community)}
          >
            <div class="community-icon">
              {community.name.charAt(0).toUpperCase()}
            </div>
            <span class={community.isHosted ? "community-name community-name-hosted" : "community-name"}>
              {community.name}
            </span>
            <Show when={unreads(community) > 0}>
              <span class="community-unread-badge">{unreads(community)}</span>
            </Show>
          </div>
        )}
      </For>
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
    </div>
  );
};

export default CommunityList;
