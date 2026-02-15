import { Component } from "solid-js";
import Modal from "./Modal";

interface ConfirmDialogProps {
  isOpen: boolean;
  title: string;
  message: string;
  danger?: boolean;
  confirmLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
}

const ConfirmDialog: Component<ConfirmDialogProps> = (props) => (
  <Modal isOpen={props.isOpen} title={props.title} onClose={props.onCancel} size="sm">
    <div class="confirm-dialog-body">
      <p>{props.message}</p>
      <div class="confirm-dialog-actions">
        <button class="settings-action-btn" onClick={props.onCancel}>Cancel</button>
        <button
          class={props.danger ? "settings-danger-btn" : "settings-save-btn"}
          onClick={props.onConfirm}
        >
          {props.confirmLabel ?? "Confirm"}
        </button>
      </div>
    </div>
  </Modal>
);

export default ConfirmDialog;
