import { Component } from "solid-js";
import SimpleInputModal from "../common/SimpleInputModal";
import { handleRenameCategory } from "../../handlers/community.handlers";

interface RenameCategoryModalProps {
  isOpen: boolean;
  communityId: string;
  categoryId: string;
  currentName: string;
  onClose: () => void;
}

const RenameCategoryModal: Component<RenameCategoryModalProps> = (props) => (
  <SimpleInputModal
    isOpen={props.isOpen}
    title="Rename Category"
    onClose={props.onClose}
    onSubmit={(name) => handleRenameCategory(props.communityId, props.categoryId, name)}
    placeholder="Category name..."
    submitLabel="Rename"
    initialValue={props.currentName}
    validate={(name) => name === props.currentName ? "Name is unchanged" : null}
  />
);

export default RenameCategoryModal;
