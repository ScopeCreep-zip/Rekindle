import { Component } from "solid-js";
import SimpleInputModal from "../common/SimpleInputModal";
import { handleCreateCommunity } from "../../handlers/community.handlers";

interface CreateCommunityModalProps {
  isOpen: boolean;
  onClose: () => void;
}

const CreateCommunityModal: Component<CreateCommunityModalProps> = (props) => (
  <SimpleInputModal
    isOpen={props.isOpen}
    title="Create Community"
    onClose={props.onClose}
    onSubmit={(name) => handleCreateCommunity(name)}
    placeholder="Community name..."
    submitLabel="Create"
  />
);

export default CreateCommunityModal;
