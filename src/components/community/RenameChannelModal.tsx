import { Component } from "solid-js";
import SimpleInputModal from "../common/SimpleInputModal";
import { handleRenameChannel } from "../../handlers/community.handlers";

interface RenameChannelModalProps {
  isOpen: boolean;
  communityId: string;
  channelId: string;
  currentName: string;
  onClose: () => void;
}

const RenameChannelModal: Component<RenameChannelModalProps> = (props) => (
  <SimpleInputModal
    isOpen={props.isOpen}
    title="Rename Channel"
    onClose={props.onClose}
    onSubmit={(name) => handleRenameChannel(props.communityId, props.channelId, name)}
    placeholder="Channel name..."
    submitLabel="Rename"
    initialValue={props.currentName}
    validate={(name) => name === props.currentName ? "Name is unchanged" : null}
  />
);

export default RenameChannelModal;
