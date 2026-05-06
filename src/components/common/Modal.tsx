import { Component, JSX, Show } from "solid-js";
import { Dialog } from "@kobalte/core/dialog";
import { ICON_CLOSE } from "../../icons";

interface ModalProps {
  isOpen: boolean;
  title: string;
  onClose: () => void;
  size?: "sm" | "md" | "lg";
  /**
   * When `false`, the Esc key, the close button, and overlay-click are
   * all suppressed. Use for takeover dialogs where the user must
   * complete an action (e.g., gated-community onboarding). Defaults
   * to `true`.
   */
  dismissable?: boolean;
  children: JSX.Element;
}

/**
 * Architecture §32 a11y — Kobalte Dialog provides WAI-ARIA APG-compliant
 * focus trap, scroll lock, `aria-modal`, return-focus-to-trigger, and
 * dismissable-layer support out of the box. Esc / overlay-click /
 * focus-outside are suppressed when `dismissable={false}`.
 */
const Modal: Component<ModalProps> = (props) => {
  const isDismissable = (): boolean => props.dismissable !== false;

  function handleOpenChange(open: boolean): void {
    if (!open && isDismissable()) {
      props.onClose();
    }
  }

  function suppressIfLocked(event: Event): void {
    if (!isDismissable()) event.preventDefault();
  }

  return (
    <Dialog open={props.isOpen} onOpenChange={handleOpenChange} modal={true}>
      <Dialog.Portal>
        <Dialog.Overlay class="modal-overlay" />
        <div class="modal-overlay-positioner">
          <Dialog.Content
            class={`modal-container modal-container-${props.size ?? "md"}`}
            onEscapeKeyDown={suppressIfLocked}
            onPointerDownOutside={suppressIfLocked}
            onInteractOutside={suppressIfLocked}
          >
            <div class="modal-header">
              <Dialog.Title class="modal-title">{props.title}</Dialog.Title>
              <Show when={isDismissable()}>
                <Dialog.CloseButton class="modal-close-btn" aria-label="Close dialog">
                  <span class="nf-icon" aria-hidden="true">{ICON_CLOSE}</span>
                </Dialog.CloseButton>
              </Show>
            </div>
            <div class="modal-body">{props.children}</div>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog>
  );
};

export default Modal;
