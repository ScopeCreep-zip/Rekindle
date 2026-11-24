import { Component, For, Show, createEffect, createSignal, on } from "solid-js";
import { Dialog } from "@kobalte/core/dialog";
import { returnFocusToOpener } from "../../utils/dialog_focus";
import { friendsState } from "../../stores/friends.store";
import { handleStartGroupCall } from "../../actions/calls.actions";

// Wave 12 W12.10 — friend multi-select dialog for starting a group
// call. Audio/video toggle + checkbox list + Call button. Closes on
// success.
/** The group a call is started for (GV2): its id, name and members, as the Host names them. */
export interface GroupCallTarget {
  groupId: string;
  groupName: string;
  members: { pubkey: string; displayName: string }[];
}

interface StartGroupCallModalProps {
  isOpen: boolean;
  onClose: () => void;
  /** Opened from a group's Call: only its members are listed, all ticked. */
  group?: GroupCallTarget | null;
}

const StartGroupCallModal: Component<StartGroupCallModalProps> = (props) => {
  const [selected, setSelected] = createSignal<Record<string, boolean>>({});
  const [video, setVideo] = createSignal(false);
  const [busy, setBusy] = createSignal(false);

  // A group's Call opens with its members ticked (the user may untick).
  createEffect(
    on(
      () => [props.isOpen, props.group] as const,
      ([open, group]) => {
        if (open && group) setSelected(Object.fromEntries(group.members.map((m) => [m.pubkey, true])));
      },
    ),
  );

  function toggle(key: string): void {
    setSelected((s) => ({ ...s, [key]: !s[key] }));
  }

  function reset(): void {
    setSelected({});
    setVideo(false);
    setBusy(false);
  }

  async function startCall(): Promise<void> {
    const sel = selected();
    const keys = Object.keys(sel).filter((k) => sel[k]);
    if (keys.length === 0) return;
    setBusy(true);
    const id = await handleStartGroupCall(keys, video(), props.group?.groupId ?? null);
    setBusy(false);
    if (id) {
      reset();
      props.onClose();
    }
  }

  // A group's call lists only its members (none can be added); otherwise
  // the friends.
  const friendList = (): { publicKey: string; displayName: string }[] => {
    const group = props.group;
    if (group) return group.members.map((m) => ({ publicKey: m.pubkey, displayName: m.displayName }));
    return Object.values(friendsState.friends).filter((f) => f.friendshipState !== "pendingOut");
  };

  return (
    <Dialog
      open={props.isOpen}
      onOpenChange={(o) => {
        if (!o) {
          reset();
          props.onClose();
        }
      }}
      modal
    >
      <Dialog.Portal>
        <Dialog.Overlay class="modal-overlay" />
        <div class="modal-overlay-positioner">
          <Dialog.Content class="modal-container modal-container-md" {...returnFocusToOpener()}>
            <div class="modal-header">
              <Dialog.Title class="modal-title">{props.group ? `Call ${props.group.groupName}` : "Start Group Call"}</Dialog.Title>
            </div>
            <div class="modal-body">
              <div class="form-row" style="gap: 12px; align-items: center;">
                <label class="settings-option" style="flex-direction: row; gap: 8px;">
                  <input
                    type="checkbox"
                    checked={video()}
                    onChange={(e) => setVideo(e.currentTarget.checked)}
                  />
                  <span>Video</span>
                </label>
                <span class="settings-hint">
                  {Object.values(selected()).filter(Boolean).length} invitee
                  {Object.values(selected()).filter(Boolean).length === 1 ? "" : "s"}
                </span>
              </div>
              <div class="start-group-call-list">
                <Show
                  when={friendList().length > 0}
                  fallback={<div class="empty-placeholder-subtitle">No friends to call</div>}
                >
                  <For each={friendList()}>
                    {(friend) => (
                      <label class="settings-option" style="flex-direction: row; gap: 8px;">
                        <input
                          type="checkbox"
                          checked={!!selected()[friend.publicKey]}
                          onChange={() => toggle(friend.publicKey)}
                        />
                        <span class="buddy-name">{friend.displayName}</span>
                      </label>
                    )}
                  </For>
                </Show>
              </div>
            </div>
            <div class="modal-footer">
              <button
                type="button"
                class="form-btn-secondary"
                onClick={() => {
                  reset();
                  props.onClose();
                }}
              >
                Cancel
              </button>
              <button
                type="button"
                class="form-btn-primary"
                disabled={
                  busy() || Object.values(selected()).filter(Boolean).length === 0
                }
                onClick={() => void startCall()}
              >
                {busy() ? "Calling…" : "Call"}
              </button>
            </div>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog>
  );
};

export default StartGroupCallModal;
