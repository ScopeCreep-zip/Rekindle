import { type Component, For, Show, createEffect, createMemo, createSignal } from "solid-js";
import { communityState } from "../../stores/community.store";
import { buddyListUI } from "../../stores/buddylist-ui.store";
import { commands } from "../../ipc/commands";
import type { Community } from "../../stores/community.store";
import ScrollArea from "../common/ScrollArea";
import { sendSkinSelection } from "../../actions/skin.actions";

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

  const hasAnyCommunities = createMemo(() => Object.keys(communityState.communities).length > 0);

  function handleDoubleClick(community: Community): void {
    commands.openCommunityWindow(community.id);
  }

  // The selected row (plan CB1): the Host keeps this window's selection, so
  // a skin's Open and Leave act on it and are disabled without one.
  const [selected, setSelected] = createSignal<string | null>(null);
  // A community no longer listed (left, removed) is no longer selected.
  createEffect(() => {
    const id = selected();
    if (id !== null && !(id in communityState.communities)) setSelected(null);
  });
  createEffect(() => {
    const id = selected();
    void sendSkinSelection(
      id !== null
        ? { kind: "community", arg: { community: id, channel: null, server: null } }
        : null
    );
  });

  return (
    <ScrollArea class="buddy-list">
      <Show
        when={hasAnyCommunities()}
        fallback={
          <div class="empty-placeholder">
            <div class="empty-placeholder-title">No Communities</div>
            <div class="empty-placeholder-subtitle">Create or join a community to get started</div>
          </div>
        }
      >
        <Show
          when={filteredCommunities().length > 0}
          fallback={
            <div class="empty-placeholder">
              <div class="empty-placeholder-subtitle">No matches</div>
            </div>
          }
        >
          {/* One listbox of the rows, laid out as the list's own children. */}
          <div class="community-listbox" role="listbox" aria-label="Communities">
            <For each={filteredCommunities()}>
              {(community) => {
                const unreads = () => communityUnreads(community);
                return (
                  <div
                    class={`community-item${selected() === community.id ? " community-item-selected" : ""}`}
                    role="option"
                    tabIndex={0}
                    aria-selected={selected() === community.id}
                    onClick={() => setSelected(community.id)}
                    onDblClick={() => handleDoubleClick(community)}
                    onKeyDown={(e) => {
                      // Enter opens, Space selects, as a double-click and a click do.
                      if (e.key === "Enter") handleDoubleClick(community);
                      else if (e.key === " ") {
                        e.preventDefault();
                        setSelected(community.id);
                      }
                    }}
                    title={`Double-click to open ${community.name}`}
                  >
                    <Show
                      when={community.iconDataUrl}
                      fallback={
                        <div class="community-icon">{community.name.charAt(0).toUpperCase()}</div>
                      }
                    >
                      {(url) => (
                        <img
                          class="community-icon community-icon-image"
                          src={url()}
                          alt={community.name}
                        />
                      )}
                    </Show>
                    <span class="community-name">{community.name}</span>
                    <Show when={unreads() > 0}>
                      <span class="buddy-unread-badge">{unreads()}</span>
                    </Show>
                  </div>
                );
              }}
            </For>
          </div>
        </Show>
      </Show>
    </ScrollArea>
  );
};

export default CommunityListCompact;
