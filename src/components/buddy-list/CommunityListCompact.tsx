import { Component, For, Show, createMemo } from "solid-js";
import { communityState } from "../../stores/community.store";
import { buddyListUI } from "../../stores/buddylist-ui.store";
import { commands } from "../../ipc/commands";
import type { Community } from "../../stores/community.store";
import ScrollArea from "../common/ScrollArea";

function communityUnreads(community: Community): number {
  let total = 0;
  for (const ch of community.channels) {
    total += ch.unreadCount;
  }
  return total;
}

const CommunityListCompact: Component = () => {
  const filteredCommunities = createMemo(() => {
    const all = Object.values(communityState.communities);
    const query = buddyListUI.searchQuery.trim().toLowerCase();
    if (!query) return all;
    return all.filter((c) => c.name.toLowerCase().includes(query));
  });

  const hasAnyCommunities = createMemo(() =>
    Object.keys(communityState.communities).length > 0,
  );

  function handleDoubleClick(community: Community): void {
    commands.openCommunityWindow(community.id, community.name);
  }

  return (
    <ScrollArea class="buddy-list">
      <Show when={hasAnyCommunities()} fallback={
        <div class="empty-placeholder">
          <div class="empty-placeholder-title">No Communities</div>
          <div class="empty-placeholder-subtitle">Create or join a community to get started</div>
        </div>
      }>
        <Show when={filteredCommunities().length > 0} fallback={
          <div class="empty-placeholder">
            <div class="empty-placeholder-subtitle">No matches</div>
          </div>
        }>
          <For each={filteredCommunities()}>
            {(community) => {
              const unreads = () => communityUnreads(community);
              return (
                <div
                  class="community-item"
                  onDblClick={() => handleDoubleClick(community)}
                  title={`Double-click to open ${community.name}`}
                >
                  <div class="community-icon">
                    {community.name.charAt(0).toUpperCase()}
                  </div>
                  <span class="community-name">{community.name}</span>
                  <Show when={unreads() > 0}>
                    <span class="buddy-unread-badge">{unreads()}</span>
                  </Show>
                </div>
              );
            }}
          </For>
        </Show>
      </Show>
    </ScrollArea>
  );
};

export default CommunityListCompact;
