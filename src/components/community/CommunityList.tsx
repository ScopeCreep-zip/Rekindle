import { Component, For, Show } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";
import { ContextMenu } from "@kobalte/core/context-menu";
import { communityState, Community } from "../../stores/community.store";

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
                    <SkinGlyph name="settings" class="context-menu-icon" />
                    Settings
                  </ContextMenu.Item>
                </Show>
                <ContextMenu.Item
                  class="context-menu-item"
                  onSelect={() => navigator.clipboard.writeText(community.id)}
                >
                  <SkinGlyph name="copy" class="context-menu-icon" />
                  Copy ID
                </ContextMenu.Item>
                <Show when={props.onLeave}>
                  <ContextMenu.Item
                    class="context-menu-item context-menu-item-danger"
                    onSelect={() => props.onLeave!(community.id)}
                  >
                    <SkinGlyph name="logout" class="context-menu-icon" />
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
