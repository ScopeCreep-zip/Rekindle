import { Component } from "solid-js";
import SimpleInputModal from "../common/SimpleInputModal";
import { friendsState, setFriendsState } from "../../stores/friends.store";
import { commands } from "../../ipc/commands";

const NewChatModal: Component = () => {
  function handleClose(): void {
    setFriendsState("showNewChat", false);
  }

  return (
    <SimpleInputModal
      isOpen={friendsState.showNewChat}
      title="New Chat"
      onClose={handleClose}
      onSubmit={(key) => commands.openChatWindow(key)}
      placeholder="Enter public key..."
      submitLabel="Start Chat"
    />
  );
};

export default NewChatModal;
