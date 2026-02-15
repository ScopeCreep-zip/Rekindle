import { Component, For, Show, createMemo, createSignal } from "solid-js";
import { Channel } from "../../stores/community.store";
import ContextMenu from "../common/ContextMenu";
import type { ContextMenuItem } from "../common/ContextMenu";
import {
  ICON_CHANNEL_TEXT,
  ICON_VOLUME_HIGH,
  ICON_PHONE,
  ICON_PENCIL,
  ICON_DELETE,
} from "../../icons";

interface ChannelListProps {
  channels: Channel[];
  selectedId?: string;
  communityId: string;
  canManage: boolean;
  onSelect: (id: string) => void;
  onVoiceJoin?: (id: string) => void;
  onRename?: (channelId: string, currentName: string) => void;
  onDelete?: (channelId: string) => void;
}

const ChannelList: Component<ChannelListProps> = (props) => {
  const textChannels = createMemo(() =>
    props.channels.filter((c) => c.type === "text")
  );

  const voiceChannels = createMemo(() =>
    props.channels.filter((c) => c.type === "voice")
  );

  const [contextMenu, setContextMenu] = createSignal<{
    x: number;
    y: number;
    channel: Channel;
  } | null>(null);

  function handleContextMenu(e: MouseEvent, channel: Channel): void {
    e.preventDefault();
    if (!props.canManage) return;
    setContextMenu({ x: e.clientX, y: e.clientY, channel });
  }

  function contextMenuItems(): ContextMenuItem[] {
    const ctx = contextMenu();
    if (!ctx) return [];
    return [
      {
        label: "Rename Channel",
        icon: ICON_PENCIL,
        action: () => {
          props.onRename?.(ctx.channel.id, ctx.channel.name);
        },
      },
      {
        label: "Delete Channel",
        icon: ICON_DELETE,
        action: () => {
          props.onDelete?.(ctx.channel.id);
        },
        danger: true,
      },
    ];
  }

  return (
    <div class="channel-list">
      <div class="channel-section-header">Text Channels</div>
      <For each={textChannels()}>
        {(channel) => (
          <div
            class={`channel-item ${props.selectedId === channel.id ? "channel-item-selected" : ""}`}
            onClick={() => props.onSelect(channel.id)}
            onContextMenu={(e) => handleContextMenu(e, channel)}
          >
            <span class="nf-icon channel-icon">{ICON_CHANNEL_TEXT}</span>
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
                onClick={() => props.onSelect(channel.id)}
                onContextMenu={(e) => handleContextMenu(e, channel)}
              >
                <span class="nf-icon channel-icon">{ICON_VOLUME_HIGH}</span>
                <span class="channel-name">{channel.name}</span>
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
              </div>
            )}
          </For>
        </>
      )}

      <Show when={contextMenu()}>
        {(menu) => (
          <ContextMenu
            items={contextMenuItems()}
            x={menu().x}
            y={menu().y}
            onClose={() => setContextMenu(null)}
          />
        )}
      </Show>
    </div>
  );
};

export default ChannelList;
