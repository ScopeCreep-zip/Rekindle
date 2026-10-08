import { Component, For, Show } from "solid-js";
import { ContextMenu } from "@kobalte/core/context-menu";
import { communityState, Community } from "../../stores/community.store";
import { ICON_SETTINGS, ICON_LOGOUT, ICON_COPY } from "../../icons";

interface CommunityListProps {
  selectedId?: string;
  onSelect: (id: string) => void;
  onSettings?: (id: string) => void;
  onLeave?: (id: string) => void;
}

const CommunityList: Component<CommunityListProps> = (props) => {
  const list = () => Object.values(communityState.communities);

  function unreads(c: Community): number {
    return c.channels.reduce((sum, ch) => sum + (ch.unreadCount ?? 0), 0);
  }

  return (
    <div class="community-list">
      <For each={list()} fallback={
        <div class="empty-placeholder">
          <div class="empty-placeholder-subtitle">No communities</div>
        </div>
      }>
        {(community: Community) => (
          <ContextMenu>
            <ContextMenu.Trigger
              as="div"
              class={`community-item ${props.selectedId === community.id ? "community-item-selected" : ""}`}
              onClick={() => props.onSelect(community.id)}
            >
              <div class="community-icon">
                {community.name.charAt(0).toUpperCase()}
              </div>
              <span class="community-name">
                {community.name}
              </span>
              <Show when={unreads(community) > 0}>
                <span class="community-unread-badge">{unreads(community)}</span>
              </Show>
            </ContextMenu.Trigger>
            <ContextMenu.Portal>
              <ContextMenu.Content class="context-menu">
                <Show when={props.onSettings}>
                  <ContextMenu.Item
                    class="context-menu-item"
                    onSelect={() => props.onSettings!(community.id)}
                  >
                    <span class="nf-icon context-menu-icon">{ICON_SETTINGS}</span>
                    Settings
                  </ContextMenu.Item>
                </Show>
                <ContextMenu.Item
                  class="context-menu-item"
                  onSelect={() => navigator.clipboard.writeText(community.id)}
                >
                  <span class="nf-icon context-menu-icon">{ICON_COPY}</span>
                  Copy ID
                </ContextMenu.Item>
                <Show when={props.onLeave}>
                  <ContextMenu.Item
                    class="context-menu-item context-menu-item-danger"
                    onSelect={() => props.onLeave!(community.id)}
                  >
                    <span class="nf-icon context-menu-icon">{ICON_LOGOUT}</span>
                    Leave
                  </ContextMenu.Item>
                </Show>
              </ContextMenu.Content>
            </ContextMenu.Portal>
          </ContextMenu>
        )}
      </For>
    </div>
  );
};

export default CommunityList;
