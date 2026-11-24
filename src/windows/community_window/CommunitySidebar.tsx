import { Component, Show } from "solid-js";
import SkinGlyph from "../../components/common/SkinGlyph";
import CommunityList from "../../components/community/CommunityList";
import ChannelList from "../../components/community/ChannelList";
import VoicePanel from "../../components/voice/VoicePanel";
import CategoryHeader from "../../components/community/CategoryHeader";
import { voiceState } from "../../stores/voice.store";
import { commands } from "../../ipc/commands";
import { handleJoinVoice } from "../../actions/voice.actions";
import {
  handleDeleteChannel,
  handleDeleteCategory,
  handleLeaveCommunity,
  handleSetNotificationOverride,
} from "../../actions/community.actions";
import type { CommunityVm } from "./useCommunityWindow";

/// Left navigation rail: community list, channel list, collapsible game
/// server + upcoming event sections, voice/video panels, and the
/// community-level search / leave actions.
const CommunitySidebar: Component<{ vm: CommunityVm }> = (props) => {
  const vm = props.vm;
  return (
    <div class="community-sidebar">
      <div class="community-sidebar-header">
        <span class="community-sidebar-title">Communities</span>
        <span class="header-btn-group">
          <button
            class="action-bar-btn header-add-btn"
            onClick={() => vm.setShowJoinCommunity(true)}
            title="Join Community"
          >
            <SkinGlyph name="room" />
          </button>
          <button
            class="action-bar-btn header-add-btn"
            onClick={() => vm.setShowCreateCommunity(true)}
            title="Create Community"
          >
            <SkinGlyph name="plus" />
          </button>
        </span>
      </div>
      <CommunityList
        selectedId={vm.selectedCommunityId()}
        onSelect={vm.handleSelectCommunity}
        onSettings={(id) => { vm.handleSelectCommunity(id); vm.setShowSettings(true); }}
        onLeave={() => void handleLeaveCommunity(vm.selectedCommunityId())}
      />
      <Show when={vm.activeCommunity()}>
        <div class="community-sidebar-header">
          <span class="community-sidebar-title">Channels</span>
          <span class="header-btn-group">
            <button
              class="action-bar-btn header-add-btn"
              onClick={() => vm.setShowSettings(true)}
              title="Community Settings"
            >
              <SkinGlyph name="settings" />
            </button>
            <Show when={vm.canManageChannels()}>
              <button
                class="action-bar-btn header-add-btn"
                onClick={() => vm.setShowCreateChannel(true)}
                title="Create Channel"
              >
                <SkinGlyph name="plus-box" />
              </button>
            </Show>
          </span>
        </div>
        <ChannelList
          channels={vm.activeCommunity()!.channels}
          categories={vm.activeCommunity()!.categories}
          members={vm.activeCommunity()!.members}
          selectedId={vm.selectedChannelId()}
          communityId={vm.selectedCommunityId()}
          canManage={vm.canManageChannels()}
          onSelect={vm.handleSelectChannel}
          onVoiceJoin={(channelId) => handleJoinVoice(channelId, vm.selectedCommunityId())}
          onRename={(channelId, currentName) => vm.setRenameTarget({ channelId, currentName })}
          onDelete={(channelId) => handleDeleteChannel(vm.selectedCommunityId(), channelId)}
          onRenameCategory={(categoryId, currentName) => vm.setRenameCategoryTarget({ categoryId, currentName })}
          onDeleteCategory={(categoryId) => handleDeleteCategory(vm.selectedCommunityId(), categoryId)}
          onCreateCategory={() => vm.setShowCreateCategory(true)}
          onSetNotification={(channelId, level) => handleSetNotificationOverride(vm.selectedCommunityId(), channelId, level)}
        />

        {/* Sidebar: Game Servers (collapsible). Shown with no servers too:
            its "All servers" link is the only way to the list and its Add
            form, so the first server must be reachable from here. */}
        <div class="sidebar-servers-section">
          <CategoryHeader
            name={`Servers (${vm.gameServers().length})`}
            isExpanded={vm.sidebarServersExpanded()}
            onToggle={() => vm.setSidebarServersExpanded(!vm.sidebarServersExpanded())}
          />
          <Show when={vm.sidebarServersExpanded()}>
            {vm.sidebarServerList().map((server) => (
              <div class="sidebar-server-row">
                <span class="sidebar-server-name">
                  {vm.gameNameCache().get(server.gameId) ?? server.label}
                </span>
                <button
                  class="sidebar-server-join-btn"
                  onClick={() => {
                    const id = parseInt(server.gameId, 10);
                    if (!isNaN(id)) void commands.launchGame(id, server.address, "community");
                  }}
                >
                  Join
                </button>
              </div>
            ))}
            <button
              class="sidebar-section-more"
              onClick={() => { vm.setShowServers(true); vm.setShowEvents(false); }}
            >
              All servers
            </button>
          </Show>
        </div>

        {/* Sidebar: Upcoming Events (collapsible). Shown with no upcoming
            event too: "All events" opens the panel that creates the first. */}
        <div class="sidebar-events-section">
          <CategoryHeader
            name="Upcoming"
            isExpanded={vm.sidebarEventsExpanded()}
            onToggle={() => vm.setSidebarEventsExpanded(!vm.sidebarEventsExpanded())}
          />
          <Show when={vm.sidebarEventsExpanded()}>
            {vm.upcomingEvents().map((event) => (
              <div class="sidebar-event-row">
                <span class="sidebar-event-title">{event.title}</span>
                <span class="sidebar-event-countdown">{vm.formatTimeUntilEvent(event.startTime)}</span>
              </div>
            ))}
            <button
              class="sidebar-section-more"
              onClick={() => { vm.setShowEvents(true); vm.setShowServers(false); }}
            >
              All events
            </button>
          </Show>
        </div>
      </Show>
      {/* Compact "connected" strip. Video, screen-share and the full control
          set now live in the main-pane CallStage (community voice channel). */}
      <Show when={voiceState.isConnected}>
        <VoicePanel />
      </Show>
      <Show when={vm.activeCommunity()}>
        <div class="community-header-actions">
          {/* Plan §Failure 9 — global community search button. Seeds
           *  the panel with `channelId=null` so SearchPanel's scope
           *  inference picks "community" (search across every
           *  channel). Cmd/Ctrl-F still scopes to the active
           *  channel as before. */}
          <button
            class="community-leave-btn"
            title="Search this community (Cmd/Ctrl+F or Cmd/Ctrl+K searches the open channel)"
            aria-label="Search community"
            onClick={() => {
              vm.setSearchInitialChannel(null);
              vm.setShowSearch(true);
            }}
          >
            <SkinGlyph name="search" /> Search
          </button>
          <button
            class="community-leave-btn"
            onClick={() => void handleLeaveCommunity(vm.selectedCommunityId())}
          >
            <SkinGlyph name="logout" /> Leave
          </button>
        </div>
      </Show>
    </div>
  );
};

export default CommunitySidebar;
