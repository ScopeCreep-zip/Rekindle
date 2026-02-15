import { Component, JSX, Show, onMount, onCleanup } from "solid-js";
import { ICON_CLOSE } from "../../icons";

interface ModalProps {
  isOpen: boolean;
  title: string;
  onClose: () => void;
  size?: "sm" | "md" | "lg";
  children: JSX.Element;
}

const Modal: Component<ModalProps> = (props) => {
  function handleOverlayClick(e: MouseEvent) {
    if (e.target === e.currentTarget) {
      props.onClose();
    }
  }

  function handleKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape" && props.isOpen) {
      props.onClose();
    }
  }

  onMount(() => {
    document.addEventListener("keydown", handleKeyDown);
  });

  onCleanup(() => {
    document.removeEventListener("keydown", handleKeyDown);
  });

  return (
    <Show when={props.isOpen}>
      <div class="modal-overlay" onClick={handleOverlayClick}>
        <div class={`modal-container modal-container-${props.size ?? "md"}`}>
          <div class="modal-header">
            <span class="modal-title">{props.title}</span>
            <button class="modal-close-btn" onClick={props.onClose}>
              <span class="nf-icon">{ICON_CLOSE}</span>
            </button>
          </div>
          <div class="modal-body">{props.children}</div>
        </div>
      </div>
    </Show>
  );
};

export default Modal;
