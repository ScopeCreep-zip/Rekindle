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
      onSubmit={(key, name) => commands.openChatWindow(key, name || key.slice(0, 12) + "...")}
      placeholder="Enter public key..."
      submitLabel="Start Chat"
      secondaryPlaceholder="Display name (optional)"
    />
  );
};

export default NewChatModal;
