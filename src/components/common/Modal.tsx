import { Component, JSX, Show, onMount, onCleanup } from "solid-js";

interface ModalProps {
  isOpen: boolean;
  title: string;
  onClose: () => void;
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
        <div class="modal-container">
          <div class="modal-header">
            <span class="modal-title">{props.title}</span>
            <button class="modal-close-btn" onClick={props.onClose}>
              X
            </button>
          </div>
          <div class="modal-body">{props.children}</div>
        </div>
      </div>
    </Show>
  );
};

export default Modal;
