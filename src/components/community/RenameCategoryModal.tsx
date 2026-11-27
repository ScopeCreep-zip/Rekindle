import type { Component } from "solid-js";
import { handleRenameCategory } from "../../actions/community.actions";
import SimpleInputModal from "../common/SimpleInputModal";

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
    validate={(name) => (name === props.currentName ? "Name is unchanged" : null)}
  />
);

export default RenameCategoryModal;
