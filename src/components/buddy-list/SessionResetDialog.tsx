import { Component, Show, createSignal } from "solid-js";
import Modal from "../common/Modal";
import { commands } from "../../ipc/commands";
import { resolveSessionReset, sessionResets } from "../../stores/session-reset.store";
import { errorMessage } from "../../utils/error";

/// A peer asks to re-establish the secure session. Accepting installs new
/// keys, so the user must compare the safety number with the peer over a
/// separate channel first. The dialog cannot be dismissed: closing it is
/// not an answer, only Accept or Decline is.
const SessionResetDialog: Component = () => {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const current = () => sessionResets()[0] ?? null;

  async function answer(accept: boolean): Promise<void> {
    const request = current();
    if (!request || busy()) return;
    setBusy(true);
    setError(null);
    try {
      if (accept) {
        await commands.acceptSessionReset(request.peerPublicKey);
      } else {
        await commands.declineSessionReset(request.peerPublicKey, "user declined");
      }
      resolveSessionReset(request.peerPublicKey);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      isOpen={current() !== null}
      title="Secure session reset"
      onClose={() => undefined}
      dismissable={false}
      size="sm"
    >
      <Show when={current()}>
        {(request) => (
          <div class="consent-dialog">
            <p class="consent-dialog-text">
              {request().peerDisplayName} wants to reset your secure session.
            </p>
            <p class="consent-dialog-key">
              Safety number <code>{request().safetyNumber}</code>
            </p>
            <p class="consent-dialog-text">
              Compare this number with {request().peerDisplayName} on a different channel
              (a phone call, in person, another trusted app) before accepting. If the numbers do
              not match, decline: accepting would install someone else's keys.
            </p>
            <Show when={error()}>
              <p class="consent-dialog-error" role="alert">{error()}</p>
            </Show>
            <div class="consent-dialog-actions">
              <button class="form-btn-secondary" disabled={busy()} onClick={() => void answer(false)}>
                Decline
              </button>
              <button class="form-btn-primary" disabled={busy()} onClick={() => void answer(true)}>
                Numbers match — accept
              </button>
            </div>
          </div>
        )}
      </Show>
    </Modal>
  );
};

export default SessionResetDialog;
