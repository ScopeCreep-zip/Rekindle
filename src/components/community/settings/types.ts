// Types shared between CommunitySettingsModal and its tabs.
//
// `ConfirmOptions` lived in CommunitySettingsModal.tsx, which renders
// every tab — and each tab takes a `requestConfirm(opts: ConfirmOptions)`
// prop, so all six imported the type back out of the modal. Six of the
// twelve module cycles in this tree were that one declaration.

/** A confirmation the modal shows on a tab's behalf. */
export interface ConfirmOptions {
  title: string;
  message: string;
  confirmLabel?: string;
  action: () => void;
}
