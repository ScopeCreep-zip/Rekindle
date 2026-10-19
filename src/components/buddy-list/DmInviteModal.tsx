import { Component, For, Show } from "solid-js";
import Modal from "../common/Modal";
import { dmState, setDmState } from "../../stores/dm.store";
import { commands } from "../../ipc/commands";

/// Surfaces inbound group DM invites (architecture §27.2) until E2.6's
/// group DM dialog replaces it. Renders a modal as long as there's at
/// least one pending invite; the user accepts or declines from the list.
/// An accepted group opens in the chat window's group mode.
const DmInviteModal: Component = () => {
  const invites = () => Object.values(dmState.pendingInvites);

  const isOpen = (): boolean => invites().length > 0;

  async function accept(recordKey: string): Promise<void> {
    await commands.acceptDmInvite(recordKey);
    setDmState("pendingInvites", recordKey, undefined!);
    await commands.openGroupChatWindow(recordKey);
  }

  async function decline(recordKey: string): Promise<void> {
    await commands.declineDmInvite(recordKey);
    setDmState("pendingInvites", recordKey, undefined!);
  }

  return (
    <Modal isOpen={isOpen()} title="Group DM invites" onClose={() => undefined} size="md">
      <div class="dm-invite-list">
        <Show
          when={invites().length > 0}
          fallback={<p class="dm-invite-empty">No pending invites.</p>}
        >
          <For each={invites()}>
            {(invite) => (
              <div class="dm-invite-row">
                <div class="dm-invite-meta">
                  <strong>{invite.initiatorPseudonym}</strong>
                  <span class="dm-invite-kind">wants to add you to a group DM</span>
                  <span class="dm-invite-key" title={invite.from}>
                    from {invite.from.slice(0, 12)}…
                  </span>
                </div>
                <div class="dm-invite-actions">
                  <button class="form-btn-primary" onClick={() => void accept(invite.recordKey)}>
                    Accept
                  </button>
                  <button class="form-btn-secondary" onClick={() => void decline(invite.recordKey)}>
                    Decline
                  </button>
                </div>
              </div>
            )}
          </For>
        </Show>
      </div>
    </Modal>
  );
};

export default DmInviteModal;
