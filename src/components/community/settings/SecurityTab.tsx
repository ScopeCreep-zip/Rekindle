import { Component, createSignal } from "solid-js";
import type { Community } from "../../../stores/community.store";
import type { ConfirmOptions } from "../CommunitySettingsModal";
import { handleRotateMek } from "../../../handlers/community.handlers";
import { addToast } from "../../../stores/toast.store";
import { ICON_KEY } from "../../../icons";
import FormField from "../../common/FormField";

interface SecurityTabProps {
  community: Community;
  requestConfirm: (opts: ConfirmOptions) => void;
}

const SecurityTab: Component<SecurityTabProps> = (props) => {
  const [rotating, setRotating] = createSignal(false);

  function confirmRotateKey(): void {
    props.requestConfirm({
      title: "Rotate Encryption Key",
      message: "Generate a new Media Encryption Key? All members will automatically receive the new key.",
      confirmLabel: "Rotate",
      action: async () => {
        setRotating(true);
        try {
          await handleRotateMek(props.community.id);
          addToast("Encryption key rotated", "success");
        } finally {
          setRotating(false);
        }
      },
    });
  }

  return (
    <div class="settings-section">
      <FormField label="MEK Generation">
        <div class="settings-value">
          {props.community.mekGeneration}
          <span class="settings-hint-inline"> (higher = more recent)</span>
        </div>
      </FormField>
      <FormField label="Encryption Key Rotation">
        <div class="settings-hint">
          Rotating the encryption key generates a new Media Encryption Key (MEK).
          All members will automatically receive the new key. Messages encrypted
          with previous keys remain readable.
        </div>
        <button class="form-btn-danger" onClick={confirmRotateKey} disabled={rotating()}>
          <span class="nf-icon">{ICON_KEY}</span> {rotating() ? "Rotating..." : "Rotate Encryption Key"}
        </button>
      </FormField>
      <FormField label="Network Model">
        <div class="settings-value">
          Rotating Coordinator (P2P)
        </div>
      </FormField>
    </div>
  );
};

export default SecurityTab;
