import { Component, Show } from "solid-js";
import ChannelChat from "../../components/chat/ChannelChat";
import CallStage from "../../components/voice/call_stage/CallStage";
import EventsPanel from "../../components/community/EventsPanel";
import GameServerList from "../../components/community/GameServerList";
import ForumChannelView from "../../components/community/ForumChannelView";
import StagePanel from "../../components/community/StagePanel";
import WelcomeScreen from "../../components/community/WelcomeScreen";
import { communityState } from "../../stores/community.store";
import {
  handleAddGameServer,
  handleRemoveGameServer,
  handleSetChannelTopic,
  handleCreateForumPost,
} from "../../handlers/community.handlers";
import {
  handleJoinVoice,
  handleLeaveVoice,
  handleRequestToSpeak,
  handleRespondToSpeakRequest,
} from "../../handlers/voice.handlers";
import { ICON_PIN, ICON_THREAD, ICON_COMMUNITIES } from "../../icons";
import type { CommunityVm } from "./useCommunityWindow";

/// Center column: game-server list, events panel, welcome screen, or the
/// active channel (header + topic editing + message list / stage / forum +
/// composer). Reactive driven entirely off the `vm` view model.
const CommunityMainPane: Component<{ vm: CommunityVm }> = (props) => {
  const vm = props.vm;
  return (
    <div class="community-main" id="main-content" tabindex="-1">
      <Show when={vm.showServers() && vm.activeCommunity()}>
        <GameServerList
          servers={vm.gameServers()}
          communityId={vm.selectedCommunityId()}
          canManage={vm.canManageCommunity()}
          onRemove={handleRemoveGameServer}
          onAdd={handleAddGameServer}
        />
      </Show>
      <Show when={!vm.showServers()}>
      <Show when={vm.showEvents() && vm.activeCommunity()} fallback={
        <Show when={vm.shouldShowWelcome() && vm.activeCommunity()?.welcomeScreen} fallback={
          <Show when={vm.activeChannel()} fallback={
            <div class="empty-placeholder">
              <div class="empty-placeholder-title">Select a channel</div>
              <div class="empty-placeholder-subtitle">Choose a community and channel to start chatting</div>
            </div>
          }>
            <div class="community-channel-header">
              <span class="nf-icon community-channel-header-icon">{vm.channelHeaderIcon()}</span>
              {vm.activeChannel()!.name}
              <Show when={vm.activeChannel()?.topic || vm.canManageChannels()}>
                <Show when={vm.editingTopic()} fallback={
                  <span
                    class={`channel-topic ${vm.canManageChannels() ? "channel-topic-editable" : ""}`}
                    onClick={() => {
                      if (vm.canManageChannels()) {
                        vm.setTopicDraft(vm.activeChannel()?.topic ?? "");
                        vm.setEditingTopic(true);
                      }
                    }}
                  >
                    {vm.activeChannel()?.topic || (vm.canManageChannels() ? "Set topic..." : "")}
                  </span>
                }>
                  <input
                    class="form-input channel-topic-header-input"
                    type="text"
                    value={vm.topicDraft()}
                    onInput={(e) => vm.setTopicDraft(e.currentTarget.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        handleSetChannelTopic(vm.selectedCommunityId(), vm.selectedChannelId(), vm.topicDraft());
                        vm.setEditingTopic(false);
                      }
                      if (e.key === "Escape") {
                        vm.setEditingTopic(false);
                      }
                    }}
                    onBlur={() => {
                      handleSetChannelTopic(vm.selectedCommunityId(), vm.selectedChannelId(), vm.topicDraft());
                      vm.setEditingTopic(false);
                    }}
                    placeholder="Channel topic..."
                  />
                </Show>
              </Show>
              <Show when={vm.activeCommunity()?.description && !vm.activeChannel()?.topic && !vm.editingTopic()}>
                <span class="community-description-hint">{vm.activeCommunity()!.description}</span>
              </Show>
              <span class="header-btn-group">
                <button
                  class={`action-bar-btn header-add-btn ${vm.rightPanel() === "pins" ? "header-btn-active" : ""}`}
                  onClick={vm.handleTogglePins}
                  title="Pinned Messages"
                >
                  <span class="nf-icon">{ICON_PIN}</span>
                  <Show when={vm.pins().length > 0}>
                    <span class="channel-header-badge">{vm.pins().length}</span>
                  </Show>
                </button>
                <button
                  class={`action-bar-btn header-add-btn ${vm.rightPanel() === "threadList" || vm.rightPanel() === "thread" ? "header-btn-active" : ""}`}
                  onClick={vm.handleToggleThreadList}
                  title="Threads"
                >
                  <span class="nf-icon">{ICON_THREAD}</span>
                  <Show when={(communityState.channelThreads[vm.selectedChannelId()] ?? []).length > 0}>
                    <span class="channel-header-badge">{(communityState.channelThreads[vm.selectedChannelId()] ?? []).length}</span>
                  </Show>
                </button>
                <button
                  class={`action-bar-btn header-add-btn ${vm.rightPanel() === "members" ? "header-btn-active" : ""}`}
                  onClick={vm.handleToggleMembers}
                  title="Members"
                  aria-label={vm.rightPanel() === "members" ? "Hide members panel" : "Show members panel"}
                  aria-pressed={vm.rightPanel() === "members"}
                >
                  <span class="nf-icon" aria-hidden="true">{ICON_COMMUNITIES}</span>
                </button>
              </span>
            </div>
            <Show when={vm.isForumChannel()} fallback={
              <Show when={vm.isStageChannel()} fallback={
                <Show when={vm.isVoiceChannel()} fallback={<ChannelChat vm={vm} />}>
                  <CallStage vm={vm} />
                </Show>
              }>
                <StagePanel
                  channel={vm.activeChannel()!}
                  voiceChannel={vm.activeVoiceChannelState()}
                  members={vm.activeCommunity()!.members}
                  myPseudonymKey={vm.activeCommunity()?.myPseudonymKey ?? null}
                  isConnectedToChannel={vm.isConnectedToActiveStage()}
                  canModerate={vm.canManageMessages()}
                  canRequestToSpeak={vm.canRequestToSpeak()}
                  onJoinStage={() => void handleJoinVoice(vm.selectedChannelId(), vm.selectedCommunityId())}
                  onLeaveStage={() => void handleLeaveVoice()}
                  onRequestToSpeak={() => void handleRequestToSpeak(vm.selectedCommunityId(), vm.selectedChannelId())}
                  onApproveRequest={(requesterPseudonym) =>
                    void handleRespondToSpeakRequest(
                      vm.selectedCommunityId(),
                      vm.selectedChannelId(),
                      requesterPseudonym,
                      true,
                    )}
                  onDenyRequest={(requesterPseudonym) =>
                    void handleRespondToSpeakRequest(
                      vm.selectedCommunityId(),
                      vm.selectedChannelId(),
                      requesterPseudonym,
                      false,
                    )}
                />
              </Show>
            }>
              <ForumChannelView
                channel={vm.activeChannel()!}
                threads={communityState.channelThreads[vm.selectedChannelId()] ?? []}
                onOpenThread={vm.handleOpenThread}
                onCreatePost={async (name, body, forumTag) => {
                  const threadId = await handleCreateForumPost(
                    vm.selectedCommunityId(),
                    vm.selectedChannelId(),
                    name,
                    body,
                    forumTag,
                  );
                  if (!threadId) return;
                  const thread = (communityState.channelThreads[vm.selectedChannelId()] ?? [])
                    .find((item) => item.id === threadId);
                  if (thread) vm.handleOpenThread(thread);
                }}
              />
            </Show>
          </Show>
        }>
          <WelcomeScreen
            screen={vm.activeCommunity()!.welcomeScreen!}
            communityName={vm.activeCommunity()!.name}
            onChannelClick={(channelId) => {
              vm.handleSelectChannel(channelId);
              vm.setShowWelcomeForCommunity(null);
            }}
          />
        </Show>
      }>
        <EventsPanel
          communityId={vm.selectedCommunityId()}
          myPseudonymKey={vm.activeCommunity()?.myPseudonymKey ?? null}
          onCreateEvent={() => vm.setShowCreateEvent(true)}
          onEditEvent={(event) => { vm.setEditingEvent(event); vm.setShowCreateEvent(true); }}
        />
      </Show>
      </Show>
    </div>
  );
};

export default CommunityMainPane;
