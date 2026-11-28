// Types shared between CommunitySettingsModal and its tabs.
//
// `ConfirmOptions` is the one confirmation still shown by the modal: the
// Security tab's MEK rotation, whose `rotate_mek` is not yet a Host call
// (`shell_commands.rs` `PENDING_PORT`). Every ported command is reviewed
// by the Host's intent gate (`IntentRequired` → `IntentDialog`).

/** A confirmation the modal shows for the Security tab. */
export interface ConfirmOptions {
  title: string;
  message: string;
  confirmLabel?: string;
  action: () => void;
}
