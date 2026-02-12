import { Component, For } from "solid-js";
import { communityState, Community } from "../../stores/community.store";

interface CommunityListProps {
  selectedId?: string;
  onSelect: (id: string) => void;
}

const CommunityList: Component<CommunityListProps> = (props) => {
  const list = () => Object.values(communityState.communities);

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
          >
            <div class="community-icon">
              {community.name.charAt(0).toUpperCase()}
            </div>
            <span class="community-name">{community.name}</span>
          </div>
        )}
      </For>
    </div>
  );
};

export default CommunityList;
