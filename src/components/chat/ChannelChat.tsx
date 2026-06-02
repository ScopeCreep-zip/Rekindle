import { Component, Show } from "solid-js";
import MessageList from "./MessageList";
import MessageInput from "./MessageInput";
import { communityState } from "../../stores/community.store";
import { authState } from "../../stores/auth.store";
import {
  handleBulkDeleteChannelMessages,
  handleRetryChannelMessage,
  handleAddReaction,
  handleRemoveReaction,
  handlePinMessage,
  handleUnpinMessage,
  handleVotePoll,
  handleClosePoll,
  handleEditChannelMessage,
} from "../../handlers/community.handlers";
import type { CommunityVm } from "../../windows/community_window/useCommunityWindow";

/// The channel message list + composer for a community text (or text-in-voice)
/// channel. Extracted from CommunityMainPane so the same surface can be reused
/// inside the voice CallStage's collapsible chat column.
const ChannelChat: Component<{ vm: CommunityVm }> = (props) => {
  const vm = props.vm;
  return (
    <>
      <MessageList
        communityId={vm.selectedCommunityId()}
        channelId={vm.selectedChannelId()}
        messages={vm.channelMessages()}
        ownName={authState.displayName ?? "You"}
        peerName="Member"
        memberNames={vm.memberNames()}
        myPseudonymKey={vm.activeCommunity()?.myPseudonymKey}
        threads={communityState.channelThreads[vm.selectedChannelId()] ?? []}
        canBulkDelete={vm.canManageMessages()}
        onBulkDelete={(messageIds) =>
          handleBulkDeleteChannelMessages(
            vm.selectedCommunityId(),
            vm.selectedChannelId(),
            messageIds,
          )
        }
        onLoadOlder={vm.handleLoadOlder}
        isLoadingOlder={vm.isLoadingOlder()}
        onRetry={(messageId) => handleRetryChannelMessage(vm.selectedChannelId(), messageId)}
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
        onCreateThread={(messageId) => {
          vm.openCreateThreadModal(messageId);
        }}
        onCreatePoll={(messageId) => vm.setCreatePollTarget(messageId)}
        onOpenThread={vm.handleOpenThread}
        onEdit={(messageId, currentBody) => {
          vm.setEditState({ messageId, body: currentBody });
        }}
        onDelete={(messageId) => {
          vm.setDeleteTarget(messageId);
        }}
        onVotePoll={(pollId, selectedAnswers) =>
          handleVotePoll(vm.selectedCommunityId(), vm.selectedChannelId(), pollId, selectedAnswers)
        }
        onClosePoll={(pollId) =>
          handleClosePoll(vm.selectedCommunityId(), vm.selectedChannelId(), pollId)
        }
        onForward={(messageId) => vm.setForwardTarget(messageId)}
      />
      <Show when={vm.channelTypingUsers().length > 0}>
        <div class="typing-indicator">
          <span class="typing-dots">
            <span class="typing-label">
              {vm.channelTypingUsers().map((u) => u.displayName).join(", ")} {vm.channelTypingUsers().length === 1 ? "is" : "are"} typing...
            </span>
          </span>
        </div>
      </Show>
      <MessageInput
        communityId={vm.selectedCommunityId()}
        peerId={vm.selectedChannelId()}
        replyTo={vm.replyTo()}
        editMode={vm.editState()}
        onSend={vm.handleChannelSend}
        onTyping={vm.handleTyping}
        onDismissReply={() => vm.setReplyTo(null)}
        onEditSave={(messageId, newBody) => {
          handleEditChannelMessage(vm.selectedChannelId(), messageId, newBody);
          vm.setEditState(null);
        }}
        onEditCancel={() => vm.setEditState(null)}
        disabled={!vm.canPostInChannel()}
        disabledMessage={vm.isAnnouncementChannel() ? "Only admins can post in announcement channels" : "You don't have permission to send messages"}
        slowmodeSeconds={vm.activeChannel()?.slowmodeSeconds}
        bypassSlowmode={vm.canBypassSlowmode()}
      />
    </>
  );
};

export default ChannelChat;
