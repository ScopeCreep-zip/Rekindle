import { Component, Show } from "solid-js";
import MessageList from "../../components/chat/MessageList";
import MessageInput from "../../components/chat/MessageInput";
import { communityState } from "../../stores/community.store";
import { removalText } from "../../utils/removal_text";
import { hasPermission, servedPermissions, MANAGE_MESSAGES } from "../../utils/permissions";
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
  handleDeleteChannelMessage,
  handleAdminDeleteChannelMessage,
} from "../../actions/community.actions";
import type { CommunityVm } from "./useCommunityWindow";

/// The channel message list + composer for a community text (or text-in-voice)
/// channel. Extracted from CommunityMainPane so the same surface can be reused
/// inside the voice CallStage's collapsible chat column.
const ChannelChat: Component<{ vm: CommunityVm }> = (props) => {
  const vm = props.vm;
  // The Host serves our permissions; it re-checks MANAGE_MESSAGES on the request.
  const canManageMessages = () =>
    hasPermission(servedPermissions(communityState.communities[vm.selectedCommunityId()]), MANAGE_MESSAGES);
  return (
    <>
      <MessageList
        communityId={vm.selectedCommunityId()}
        channelId={vm.selectedChannelId()}
        messages={vm.channelMessages()}
        ownName={vm.activeCommunity()?.myDisplayName ?? "Member"}
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
          void handleDeleteChannelMessage(vm.selectedChannelId(), messageId);
        }}
        onAdminDelete={canManageMessages()
          ? (messageId) => void handleAdminDeleteChannelMessage(vm.selectedCommunityId(), vm.selectedChannelId(), messageId)
          : undefined}
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
        label={`Message #${vm.activeChannel()?.name ?? "channel"}`}
        communityId={vm.selectedCommunityId()}
        peerId={vm.selectedChannelId()}
        replyTo={vm.replyTo()}
        editMode={vm.editState()}
        onSend={vm.handleChannelSend}
        onDraftChanged={vm.handleDraftChanged}
        onDismissReply={() => vm.setReplyTo(null)}
        onEditSave={(messageId, newBody) => {
          handleEditChannelMessage(vm.selectedChannelId(), messageId, newBody);
          vm.setEditState(null);
        }}
        onEditCancel={() => vm.setEditState(null)}
        disabled={!vm.canPostInChannel()}
        disabledMessage={
          vm.ownRemoval()
            ? removalText(vm.activeCommunity()?.name ?? "", vm.ownRemoval()!)
            : vm.joinWithdrawn()
              ? vm.joinWithdrawn()!
            : vm.channelLocked() !== null
              ? vm.channelLocked()!
              : vm.isAnnouncementChannel()
              ? "Only admins can post in announcement channels"
              : "You don't have permission to send messages"
        }
        slowmodeSeconds={vm.activeChannel()?.slowmodeSeconds}
        bypassSlowmode={vm.canBypassSlowmode()}
      />
    </>
  );
};

export default ChannelChat;
