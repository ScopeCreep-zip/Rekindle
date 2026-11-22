import { Component, For, Show, createEffect, createSignal, on } from "solid-js";
import { Dialog } from "@kobalte/core/dialog";
import { returnFocusToOpener } from "../../utils/dialog_focus";
import { friendsState } from "../../stores/friends.store";
import { buddyListUI, setBuddyListUI } from "../../stores/buddylist-ui.store";
import { handleCreateGroupDm } from "../../actions/group-dm.actions";

// "New group" (plan E2.6): pick 2–7 friends and a name. The Host checks
// the roster and the name and shows the review (Consequential); the
// window only keeps Create disabled outside the member range it states.
const MIN_PICKED = 2;
const MAX_PICKED = 7;
const MAX_NAME_CHARS = 64;

const NewGroupModal: Component = () => {
  const [name, setName] = createSignal("");
  const [picked, setPicked] = createSignal<string[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  // Every friend the Host lists; one it cannot add yet is shown disabled
  // with the Host's reason, never left out (GC3).
  const friends = () => Object.values(friendsState.friends);
  const count = () => picked().length;
  const ready = () =>
    !busy() && name().trim().length > 0 && count() >= MIN_PICKED && count() <= MAX_PICKED;

  // A skin's New Room opens the dialog with its selected friend picked.
  createEffect(
    on(
      () => buddyListUI.showNewGroup,
      (open) => {
        const peer = buddyListUI.newGroupPeer;
        if (open && peer) setPicked([peer]);
      },
    ),
  );

  function toggle(key: string): void {
    setPicked((p) => (p.includes(key) ? p.filter((k) => k !== key) : [...p, key]));
  }

  function close(): void {
    setName("");
    setPicked([]);
    setBusy(false);
    setError(null);
    setBuddyListUI({ showNewGroup: false, newGroupPeer: null });
  }

  async function create(e: Event): Promise<void> {
    e.preventDefault();
    if (!ready()) return;
    setBusy(true);
    setError(null);
    const err = await handleCreateGroupDm(name().trim(), picked());
    setBusy(false);
    if (err) setError(err);
    else close();
  }

  return (
    <Dialog open={buddyListUI.showNewGroup} onOpenChange={(o) => !o && close()} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="modal-overlay" />
        <div class="modal-overlay-positioner">
          <Dialog.Content class="modal-container modal-container-md" {...returnFocusToOpener()}>
            <div class="modal-header">
              <Dialog.Title class="modal-title">New Group</Dialog.Title>
            </div>
            <form class="modal-body" onSubmit={(e) => void create(e)}>
              <label class="form-label" for="new-group-name">Group name</label>
              <input
                id="new-group-name"
                class="form-input"
                type="text"
                maxLength={MAX_NAME_CHARS}
                value={name()}
                onInput={(e) => setName(e.currentTarget.value)}
              />
              <div class="group-dm-picker-hint" id="new-group-hint">
                Choose {MIN_PICKED}–{MAX_PICKED} friends ({count()} chosen)
              </div>
              <div class="group-dm-picker" role="group" aria-label="Group members" aria-describedby="new-group-hint">
                <Show
                  when={friends().length > 0}
                  fallback={<div class="empty-placeholder-subtitle">No friends to add</div>}
                >
                  <For each={friends()}>
                    {(friend) => (
                      <label class="group-dm-picker-option">
                        <input
                          type="checkbox"
                          checked={picked().includes(friend.publicKey)}
                          disabled={friend.groupDmBlock !== null}
                          aria-describedby={friend.groupDmBlock ? `ngb-${friend.publicKey}` : undefined}
                          onChange={() => toggle(friend.publicKey)}
                        />
                        <span class="buddy-name">{friend.displayName}</span>
                        <Show when={friend.groupDmBlock}>
                          {(reason) => (
                            <span class="group-dm-picker-reason" id={`ngb-${friend.publicKey}`}>
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
                <button type="submit" class="form-btn-primary" disabled={!ready()}>
                  {busy() ? "Creating…" : "Create"}
                </button>
              </div>
            </form>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog>
  );
};

export default NewGroupModal;
