import { Component, Show } from "solid-js";
import ForwardMessageDialog from "../../components/chat/ForwardMessageDialog";
import CreateCommunityModal from "../../components/community/CreateCommunityModal";
import CreateChannelModal from "../../components/community/CreateChannelModal";
import JoinCommunityModal from "../../components/community/JoinCommunityModal";
import CommunitySettingsModal from "../../components/community/CommunitySettingsModal";
import RenameChannelModal from "../../components/community/RenameChannelModal";
import RenameCategoryModal from "../../components/community/RenameCategoryModal";
import CreateCategoryModal from "../../components/community/CreateCategoryModal";
import CreateEventModal from "../../components/community/CreateEventModal";
import CreatePollModal from "../../components/community/CreatePollModal";
import OnboardingWizard from "../../components/community/OnboardingWizard";
import SearchPanel from "../../components/chat/SearchPanel";
import CreateThreadModal from "../../components/community/CreateThreadModal";
import ConfirmDialog from "../../components/common/ConfirmDialog";
import { setCommunityState } from "../../stores/community.store";
import {
  handleLeaveCommunity,
  handleDeleteChannelMessage,
  handleSubmitOnboarding,
} from "../../actions/community.actions";
import type { CommunityVm } from "./useCommunityWindow";

/// All modal / overlay dialogs for `CommunityWindow`, rendered as a flat
/// sibling group so each manages its own open state off the `vm` signals.
const CommunityModals: Component<{ vm: CommunityVm }> = (props) => {
  const vm = props.vm;
  return (
    <>
      <CreateCommunityModal
        isOpen={vm.showCreateCommunity()}
        onClose={() => vm.setShowCreateCommunity(false)}
      />
      <JoinCommunityModal
        isOpen={vm.showJoinCommunity()}
        onClose={() => vm.setShowJoinCommunity(false)}
      />
      <Show when={vm.activeCommunity()}>
        <CreateChannelModal
          isOpen={vm.showCreateChannel()}
          communityId={vm.selectedCommunityId()}
          onClose={() => vm.setShowCreateChannel(false)}
        />
        <CommunitySettingsModal
          isOpen={vm.showSettings()}
          community={vm.activeCommunity()!}
          myRoleIds={vm.myRoleIds()}
          onClose={() => vm.setShowSettings(false)}
        />
        <Show when={vm.showSearch()}>
          <SearchPanel
            communityId={vm.selectedCommunityId()}
            channelId={vm.searchInitialChannel()}
            onClose={() => vm.setShowSearch(false)}
          />
        </Show>
        <CreateCategoryModal
          isOpen={vm.showCreateCategory()}
          communityId={vm.selectedCommunityId()}
          onClose={() => vm.setShowCreateCategory(false)}
        />
        <CreateEventModal
          isOpen={vm.showCreateEvent()}
          communityId={vm.selectedCommunityId()}
          onClose={() => { vm.setShowCreateEvent(false); vm.setEditingEvent(null); }}
          isEditing={vm.editingEvent() !== null}
          eventId={vm.editingEvent()?.id}
          initialTitle={vm.editingEvent()?.title}
          initialDescription={vm.editingEvent()?.description}
          initialStartTime={vm.editingEvent()?.startTime}
          initialEndTime={vm.editingEvent()?.endTime ?? undefined}
          initialMaxAttendees={vm.editingEvent()?.maxAttendees ?? undefined}
        />
        <CreatePollModal
          isOpen={vm.createPollTarget() !== null}
          communityId={vm.selectedCommunityId()}
          channelId={vm.selectedChannelId()}
          messageId={vm.createPollTarget() ?? ""}
          onClose={() => vm.setCreatePollTarget(null)}
        />
        <CreateThreadModal
          isOpen={vm.createThreadTarget() !== null}
          initialName={vm.createThreadTarget()?.initialName ?? ""}
          onClose={() => vm.setCreateThreadTarget(null)}
          onSubmit={async (name, autoArchive) => {
            try {
              await vm.handleSubmitCreateThread(name, autoArchive);
              vm.setCreateThreadTarget(null);
            } catch (e) {
              console.error("Failed to create thread:", e);
            }
          }}
        />
        <Show when={vm.forwardTarget()}>
          {(messageId) => (
            <ForwardMessageDialog
              sourceCommunityId={vm.selectedCommunityId()}
              sourceChannelId={vm.selectedChannelId()}
              sourceMessageId={messageId()}
              onClose={() => vm.setForwardTarget(null)}
            />
          )}
        </Show>
      </Show>
      <Show when={vm.renameTarget()}>
        {(target) => (
          <RenameChannelModal
            isOpen={true}
            communityId={vm.selectedCommunityId()}
            channelId={target().channelId}
            currentName={target().currentName}
            onClose={() => vm.setRenameTarget(null)}
          />
        )}
      </Show>
      <Show when={vm.renameCategoryTarget()}>
        {(target) => (
          <RenameCategoryModal
            isOpen={true}
            communityId={vm.selectedCommunityId()}
            categoryId={target().categoryId}
            currentName={target().currentName}
            onClose={() => vm.setRenameCategoryTarget(null)}
          />
        )}
      </Show>
      <ConfirmDialog
        isOpen={vm.showLeaveConfirm()}
        title="Leave Community"
        message={`Leave ${vm.activeCommunity()?.name ?? "this community"}? You will need to be re-invited to rejoin.`}
        danger
        confirmLabel="Leave"
        onConfirm={() => {
          handleLeaveCommunity(vm.selectedCommunityId());
          vm.setShowLeaveConfirm(false);
        }}
        onCancel={() => vm.setShowLeaveConfirm(false)}
      />
      <ConfirmDialog
        isOpen={vm.deleteTarget() !== null}
        title="Delete Message"
        message="Are you sure you want to delete this message? This cannot be undone."
        danger
        confirmLabel="Delete"
        onConfirm={() => {
          const msgId = vm.deleteTarget();
          if (msgId) {
            handleDeleteChannelMessage(vm.selectedChannelId(), msgId);
          }
          vm.setDeleteTarget(null);
        }}
        onCancel={() => vm.setDeleteTarget(null)}
      />
      <Show when={vm.shouldShowOnboarding() && vm.activeCommunity()?.onboardingConfig}>
        <OnboardingWizard
          communityId={vm.selectedCommunityId()}
          config={vm.activeCommunity()!.onboardingConfig!}
          onComplete={() => {
            setCommunityState("communities", vm.selectedCommunityId(), "onboardingComplete", true);
            if (vm.activeCommunity()?.welcomeScreen) {
              vm.setShowWelcomeForCommunity(vm.selectedCommunityId());
            }
          }}
          onCancel={
            vm.activeCommunity()!.onboardingConfig!.mode === "gated"
              ? undefined
              : () => {
                  void handleSubmitOnboarding(vm.selectedCommunityId(), []);
                }
          }
        />
      </Show>
    </>
  );
};

export default CommunityModals;
