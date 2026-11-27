import type { Component } from "solid-js";
import { handleRenameChannel } from "../../actions/community.actions";
import SimpleInputModal from "../common/SimpleInputModal";

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
    validate={(name) => (name === props.currentName ? "Name is unchanged" : null)}
  />
);

export default RenameChannelModal;
