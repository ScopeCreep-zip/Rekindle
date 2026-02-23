import { Component } from "solid-js";
import type { Community } from "../../../stores/community.store";
import type { ConfirmOptions } from "../CommunitySettingsModal";
import { handleRotateMek } from "../../../handlers/community.handlers";
import { ICON_KEY } from "../../../icons";
import FormField from "../../common/FormField";

interface SecurityTabProps {
  community: Community;
  requestConfirm: (opts: ConfirmOptions) => void;
}

const SecurityTab: Component<SecurityTabProps> = (props) => {
  function confirmRotateKey(): void {
    props.requestConfirm({
      title: "Rotate Encryption Key",
      message: "Generate a new Media Encryption Key? All members will automatically receive the new key.",
      confirmLabel: "Rotate",
      action: () => handleRotateMek(props.community.id),
    });
  }

  return (
    <div class="settings-section">
      <FormField label="MEK Generation">
        <div class="settings-value">{props.community.mekGeneration}</div>
      </FormField>
      <FormField label="Encryption Key Rotation">
        <div class="settings-hint">
          Rotating the encryption key generates a new Media Encryption Key (MEK).
          All members will automatically receive the new key. Messages encrypted
          with previous keys remain readable.
        </div>
        <button class="form-btn-danger" onClick={confirmRotateKey}>
          <span class="nf-icon">{ICON_KEY}</span> Rotate Encryption Key
        </button>
      </FormField>
      <FormField label="Server Status">
        <div class="settings-value">
          {props.community.isHosted ? "Hosted by you" : "Remote server"}
        </div>
      </FormField>
    </div>
  );
};

export default SecurityTab;
