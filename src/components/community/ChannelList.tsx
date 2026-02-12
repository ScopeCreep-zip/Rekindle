import { Component, For, createMemo } from "solid-js";
import { Channel } from "../../stores/community.store";

interface ChannelListProps {
  channels: Channel[];
  selectedId?: string;
  onSelect: (id: string) => void;
  onVoiceJoin?: (id: string) => void;
}

const ChannelList: Component<ChannelListProps> = (props) => {
  const textChannels = createMemo(() =>
    props.channels.filter((c) => c.type === "text")
  );

  const voiceChannels = createMemo(() =>
    props.channels.filter((c) => c.type === "voice")
  );

  return (
    <div class="channel-list">
      <div class="channel-section-header">Text Channels</div>
      <For each={textChannels()}>
        {(channel) => (
          <div
            class={`channel-item ${props.selectedId === channel.id ? "channel-item-selected" : ""}`}
            onClick={() => props.onSelect(channel.id)}
          >
            <span class="channel-icon">#</span>
            <span class="channel-name">{channel.name}</span>
            {channel.unreadCount > 0 && (
              <span class="channel-unread-badge">{channel.unreadCount}</span>
            )}
          </div>
        )}
      </For>

      {voiceChannels().length > 0 && (
        <>
          <div class="channel-section-header">Voice Channels</div>
          <For each={voiceChannels()}>
            {(channel) => (
              <div
                class={`channel-item ${props.selectedId === channel.id ? "channel-item-selected" : ""}`}
                onClick={() => {
                  props.onSelect(channel.id);
                  props.onVoiceJoin?.(channel.id);
                }}
              >
                <span class="channel-icon">V</span>
                <span class="channel-name">{channel.name}</span>
              </div>
            )}
          </For>
        </>
      )}
    </div>
  );
};

export default ChannelList;
