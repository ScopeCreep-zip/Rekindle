import { Component, Show } from "solid-js";
import MessageList from "../../components/chat/MessageList";
import MessageInput from "../../components/chat/MessageInput";
import { SlotMount } from "../../components/skin-tree/slots";
import { skinShell } from "../../hooks/useSkin";
import { communityState } from "../../stores/community.store";
import { removalText } from "../../utils/removal_text";
import { channelDeniedFor } from "../../utils/denied";
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
  // The Host's verdict for this channel (CAPR `manageMessages`, judged with
  // its overwrites): the moderator controls are drawn either way, disabled
  // with the reason. The author's rank is per post, answered on the request.
  const manageDenied = () =>
    channelDeniedFor(communityState.communities[vm.selectedCommunityId()], vm.selectedChannelId(), "manageMessages");
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
        bulkDeleteDenied={manageDenied()}
        adminDeleteDenied={manageDenied()}
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
        onAdminDelete={(messageId) => void handleAdminDeleteChannelMessage(vm.selectedCommunityId(), vm.selectedChannelId(), messageId)}
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
      {/* In the pack's frame (plan SKW1) the composer stands in its edit
          slot and the pack's Send art is the control (as in a chat window). */}
      <SlotMount slot="chatEdit">
      <MessageInput
        label={`Message #${vm.activeChannel()?.name ?? "channel"}`}
        sendButton={skinShell("community") !== undefined}
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
      </SlotMount>
    </>
  );
};

export default ChannelChat;
