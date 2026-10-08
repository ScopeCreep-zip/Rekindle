import { Component, For, Show } from "solid-js";
import type { Channel, Community } from "../../../stores/community.store";
import type { ConfirmOptions } from "./types";
import CategoryManager from "./channels_tab/CategoryManager";
import ChannelRow from "./channels_tab/ChannelRow";
import NewChannelForm from "./channels_tab/NewChannelForm";

interface ChannelsTabProps {
  community: Community;
  canManageChannels: boolean;
  requestConfirm: (opts: ConfirmOptions) => void;
}

const ChannelsTab: Component<ChannelsTabProps> = (props) => {
  // Group channels under their category (sorted), with uncategorized last.
  const channelGroups = (): { label: string; channels: Channel[] }[] => {
    const categories = props.community.categories;
    const channels = props.community.channels;
    const groups: { label: string; channels: Channel[] }[] = [];

    const catMap = new Map(categories.map((c) => [c.id, c.name]));
    const grouped = new Map<string | undefined, Channel[]>();
    for (const ch of channels) {
      const key = ch.categoryId;
      const arr = grouped.get(key) ?? [];
      arr.push(ch);
      grouped.set(key, arr);
    }

    for (const cat of [...categories].sort((a, b) => a.sortOrder - b.sortOrder)) {
      const chs = grouped.get(cat.id);
      if (chs?.length) {
        groups.push({ label: catMap.get(cat.id) ?? cat.id, channels: chs });
        grouped.delete(cat.id);
      }
    }

    const uncategorized = [
      ...(grouped.get(undefined) ?? []),
      ...[...grouped.entries()].filter(([k]) => k !== undefined).flatMap(([, v]) => v),
    ];
    if (uncategorized.length) {
      groups.push({
        label: categories.length > 0 ? "(Uncategorized)" : "All Channels",
        channels: uncategorized,
      });
    }

    return groups;
  };

  return (
    <div class="settings-section">
      <Show when={props.canManageChannels}>
        <CategoryManager community={props.community} requestConfirm={props.requestConfirm} />
      </Show>

      <h4 class="settings-subsection-title">Channels</h4>
      <For each={channelGroups()}>
        {(group) => (
          <>
            <div class="channel-category-label">{group.label}</div>
            <For each={group.channels}>
              {(channel) => (
                <ChannelRow
                  community={props.community}
                  channel={channel}
                  canManageChannels={props.canManageChannels}
                  requestConfirm={props.requestConfirm}
                />
              )}
            </For>
          </>
        )}
      </For>

      <Show when={props.canManageChannels}>
        <NewChannelForm communityId={props.community.id} />
      </Show>
    </div>
  );
};

export default ChannelsTab;
