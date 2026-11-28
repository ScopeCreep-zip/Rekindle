import type { Component } from "solid-js";
import Modal from "../common/Modal";
import { handleToggleCreateCommunity, handleToggleJoinCommunity } from "./BottomActionBar";

/**
 * The Communities view's New (plan CB1): the skin's third button in that
 * view's row starts a community either way, so it asks which: create one,
 * or join one with an invite. Each opens the existing panel.
 */
const CommunityStartModal: Component<{ isOpen: boolean; onClose: () => void }> = (props) => {
  function choose(open: () => void): void {
    props.onClose();
    open();
  }

  return (
    <Modal isOpen={props.isOpen} onClose={props.onClose} title="New community" size="sm">
      <div class="community-start-choices">
        <button
          type="button"
          class="form-btn-primary"
          onClick={() => choose(handleToggleCreateCommunity)}
        >
          Create a community
        </button>
        <button
          type="button"
          class="form-btn-secondary"
          onClick={() => choose(handleToggleJoinCommunity)}
        >
          Join with an invite
        </button>
      </div>
    </Modal>
  );
};

export default CommunityStartModal;
