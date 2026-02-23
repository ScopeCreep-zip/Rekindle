import { Component, Show, createSignal, createEffect, on } from "solid-js";
import Modal from "../common/Modal";
import { friendsState, setFriendsState } from "../../stores/friends.store";
import { handleLoadOutgoingInvites } from "../../handlers/buddy.handlers";
import InviteLinkTab from "./InviteLinkTab";
import PublicKeyTab from "./PublicKeyTab";

type Tab = "invite" | "key";

const AddFriendModal: Component = () => {
  const [tab, setTab] = createSignal<Tab>("invite");

  createEffect(
    on(
      () => friendsState.showAddFriend,
      (isOpen) => {
        if (isOpen && tab() === "invite") {
          handleLoadOutgoingInvites();
        }
      },
    ),
  );

  createEffect(
    on(tab, (currentTab) => {
      if (friendsState.showAddFriend && currentTab === "invite") {
        handleLoadOutgoingInvites();
      }
    }),
  );

  function handleClose(): void {
    setFriendsState("showAddFriend", false);
  }

  return (
    <Modal
      isOpen={friendsState.showAddFriend}
      title="Add Friend"
      onClose={handleClose}
    >
      <div class="form-tabs-equal">
        <button
          class="form-tab"
          classList={{ active: tab() === "invite" }}
          onClick={() => setTab("invite")}
        >
          Invite Link
        </button>
        <button
          class="form-tab"
          classList={{ active: tab() === "key" }}
          onClick={() => setTab("key")}
        >
          Public Key
        </button>
      </div>

      <Show when={tab() === "invite"}>
        <InviteLinkTab onClose={handleClose} />
      </Show>
      <Show when={tab() === "key"}>
        <PublicKeyTab onClose={handleClose} />
      </Show>
    </Modal>
  );
};

export default AddFriendModal;
