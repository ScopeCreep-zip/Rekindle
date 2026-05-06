import { Component, For, Show } from "solid-js";
import Modal from "../common/Modal";
import { dmState } from "../../stores/dm.store";
import { handleAcceptDm, handleDeclineDm } from "../../handlers/dm.handlers";

/// Surfaces inbound DM invites (architecture §27). Renders a modal as
/// long as there's at least one pending invite; the user accepts or
/// declines from the list.
const DmInviteModal: Component = () => {
  const invites = (): { recordKey: string; from: string; initiatorPseudonym: string; isGroup: boolean }[] =>
    Object.values(dmState.pendingInvites);

  const isOpen = (): boolean => invites().length > 0;

  return (
    <Modal isOpen={isOpen()} title="Direct message invites" onClose={() => undefined} size="md">
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
                  <span class="dm-invite-kind">
                    {invite.isGroup ? "wants to add you to a group DM" : "wants to start a DM"}
                  </span>
                  <span class="dm-invite-key" title={invite.from}>
                    from {invite.from.slice(0, 12)}…
                  </span>
                </div>
                <div class="dm-invite-actions">
                  <button
                    class="form-btn-primary"
                    onClick={() => handleAcceptDm(invite.recordKey)}
                  >
                    Accept
                  </button>
                  <button
                    class="form-btn-secondary"
                    onClick={() => handleDeclineDm(invite.recordKey)}
                  >
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
