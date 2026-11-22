import { Component, For, Show, createSignal } from "solid-js";
import { Dialog } from "@kobalte/core/dialog";
import { returnFocusToOpener } from "../../utils/dialog_focus";
import { friendsState } from "../../stores/friends.store";
import { groupDmState } from "../../stores/group-dm.store";
import { handleInviteToGroupDm } from "../../actions/group-dm.actions";

// "Invite to room" (plan E7.2 G8): pick friends to add to a group we
// created. Every friend not already in it is listed; one the Host cannot
// add is shown disabled with its reason (GC3). The roster cap and who may
// invite are the Host's: its review shows the invite, its refusal shows here.
const InviteToRoomModal: Component<{ groupId: string | null; onClose: () => void }> = (props) => {
  const [picked, setPicked] = createSignal<string[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const group = () => (props.groupId ? groupDmState.groups[props.groupId] : undefined);
  const inGroup = (key: string): boolean => !!group()?.members.some((m) => m.key === key);
  const friends = () => Object.values(friendsState.friends).filter((f) => !inGroup(f.publicKey));

  function toggle(key: string): void {
    setPicked((p) => (p.includes(key) ? p.filter((k) => k !== key) : [...p, key]));
  }

  function close(): void {
    setPicked([]);
    setBusy(false);
    setError(null);
    props.onClose();
  }

  async function invite(e: Event): Promise<void> {
    e.preventDefault();
    const id = props.groupId;
    if (!id || busy() || picked().length === 0) return;
    setBusy(true);
    setError(null);
    const err = await handleInviteToGroupDm(id, picked());
    setBusy(false);
    if (err) setError(err);
    else close();
  }

  return (
    <Dialog open={props.groupId !== null} onOpenChange={(o) => !o && close()} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="modal-overlay" />
        <div class="modal-overlay-positioner">
          <Dialog.Content class="modal-container modal-container-md" {...returnFocusToOpener()}>
            <div class="modal-header">
              <Dialog.Title class="modal-title">Invite to {group()?.name ?? "room"}</Dialog.Title>
            </div>
            <form class="modal-body" onSubmit={(e) => void invite(e)}>
              <div class="group-dm-picker-hint" id="invite-room-hint">
                Choose friends to invite ({picked().length} chosen). They will not see earlier messages.
              </div>
              <div class="group-dm-picker" role="group" aria-label="Friends to invite" aria-describedby="invite-room-hint">
                <Show
                  when={friends().length > 0}
                  fallback={<div class="empty-placeholder-subtitle">Every friend is already in this room</div>}
                >
                  <For each={friends()}>
                    {(friend) => (
                      <label class="group-dm-picker-option">
                        <input
                          type="checkbox"
                          checked={picked().includes(friend.publicKey)}
                          disabled={friend.groupDmBlock !== null}
                          aria-describedby={friend.groupDmBlock ? `irb-${friend.publicKey}` : undefined}
                          onChange={() => toggle(friend.publicKey)}
                        />
                        <span class="buddy-name">{friend.displayName}</span>
                        <Show when={friend.groupDmBlock}>
                          {(reason) => (
                            <span class="group-dm-picker-reason" id={`irb-${friend.publicKey}`}>
                              {reason()}
                            </span>
                          )}
                        </Show>
                      </label>
                    )}
                  </For>
                </Show>
              </div>
              <Show when={error()}>
                <div class="form-error" role="alert">{error()}</div>
              </Show>
              <div class="modal-footer">
                <button type="button" class="form-btn-secondary" onClick={close}>
                  Cancel
                </button>
                <button type="submit" class="form-btn-primary" disabled={busy() || picked().length === 0}>
                  {busy() ? "Inviting…" : "Invite"}
                </button>
              </div>
            </form>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog>
  );
};

export default InviteToRoomModal;
