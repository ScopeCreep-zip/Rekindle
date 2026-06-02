import { Component, Show, Switch, Match } from "solid-js";
import MemberList from "../../components/community/MemberList";
import ThreadPanel from "../../components/community/ThreadPanel";
import ThreadListPanel from "../../components/community/ThreadListPanel";
import PinnedMessagesPanel from "../../components/community/PinnedMessagesPanel";
import { communityState } from "../../stores/community.store";
import {
  handlePinMessage,
  handleUnpinMessage,
  handleSendThreadMessage,
  handleAddReaction,
  handleRemoveReaction,
  handleVotePoll,
  handleClosePoll,
} from "../../handlers/community.handlers";
import type { CommunityVm } from "./useCommunityWindow";

/// Phase 3 unified right rail: members, pinned messages, thread list, or
/// the open thread, switched on `vm.rightPanel()`.
const CommunityRightPanel: Component<{ vm: CommunityVm }> = (props) => {
  const vm = props.vm;
  return (
    <Show when={vm.activeCommunity()}>
      <div class="right-panel">
        <Switch>
          <Match when={vm.rightPanel() === "members"}>
            <MemberList
              members={vm.activeCommunity()!.members}
              communityId={vm.selectedCommunityId()}
              myRoleIds={vm.myRoleIds()}
              roles={vm.activeCommunity()!.roles}
              myPseudonymKey={vm.activeCommunity()?.myPseudonymKey ?? null}
            />
          </Match>
          <Match when={vm.rightPanel() === "pins"}>
            <PinnedMessagesPanel
              pins={vm.pins()}
              messages={vm.channelMessages()}
              onClose={() => vm.setRightPanel("members")}
              onUnpin={(messageId) => {
                handleUnpinMessage(vm.selectedCommunityId(), vm.selectedChannelId(), messageId);
                vm.setPins((prev) => prev.filter((p) => p.messageId !== messageId));
              }}
              onJumpToMessage={vm.handleJumpToMessage}
            />
          </Match>
          <Match when={vm.rightPanel() === "threadList"}>
            <ThreadListPanel
              threads={communityState.channelThreads[vm.selectedChannelId()] ?? []}
              onSelectThread={(threadId) => {
                const threads = communityState.channelThreads[vm.selectedChannelId()] ?? [];
                const thread = threads.find((t) => t.id === threadId);
                if (thread) vm.handleOpenThread(thread);
              }}
              onClose={() => vm.setRightPanel("members")}
            />
          </Match>
          <Match when={vm.rightPanel() === "thread"}>
            <ThreadPanel
              thread={vm.activeThread()}
              communityId={vm.selectedCommunityId()}
              messages={vm.threadMessages()}
              onClose={vm.handleCloseThread}
              onSend={handleSendThreadMessage}
              onArchive={vm.handleArchiveActiveThread}
              onReply={vm.handleReply}
              onReaction={(messageId, emoji) => handleAddReaction(vm.selectedCommunityId(), vm.selectedChannelId(), messageId, emoji)}
              onRemoveReaction={(messageId, emoji) => handleRemoveReaction(vm.selectedCommunityId(), vm.selectedChannelId(), messageId, emoji)}
              onPin={(messageId) => {
                const msg = vm.channelMessages().find((m) => m.serverMessageId === messageId);
                if (msg?.pinned) {
                  handleUnpinMessage(vm.selectedCommunityId(), vm.selectedChannelId(), messageId);
                } else {
                  handlePinMessage(vm.selectedCommunityId(), vm.selectedChannelId(), messageId);
                }
              }}
              onCreatePoll={(messageId) => vm.setCreatePollTarget(messageId)}
              onEdit={(messageId, currentBody) => vm.setEditState({ messageId, body: currentBody })}
              onDelete={(messageId) => vm.setDeleteTarget(messageId)}
              onVotePoll={(pollId, selectedAnswers) =>
                handleVotePoll(vm.selectedCommunityId(), vm.selectedChannelId(), pollId, selectedAnswers)
              }
              onClosePoll={(pollId) =>
                handleClosePoll(vm.selectedCommunityId(), vm.selectedChannelId(), pollId)
              }
              myPseudonymKey={vm.activeCommunity()?.myPseudonymKey}
            />
          </Match>
        </Switch>
      </div>
    </Show>
  );
};

export default CommunityRightPanel;
