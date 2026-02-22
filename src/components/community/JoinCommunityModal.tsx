import { Component } from "solid-js";
import SimpleInputModal from "../common/SimpleInputModal";
import { handleJoinCommunity } from "../../handlers/community.handlers";

interface JoinCommunityModalProps {
  isOpen: boolean;
  onClose: () => void;
}

const JoinCommunityModal: Component<JoinCommunityModalProps> = (props) => (
  <SimpleInputModal
    isOpen={props.isOpen}
    title="Join Community"
    onClose={props.onClose}
    onSubmit={(id, name) => handleJoinCommunity(id, name || id.slice(0, 12) + "...")}
    placeholder="Community ID..."
    submitLabel="Join"
    secondaryPlaceholder="Name (optional)"
  />
);

export default JoinCommunityModal;
