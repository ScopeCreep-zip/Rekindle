import { Component, Match, Show, Switch, createSignal } from "solid-js";
import Modal from "../common/Modal";
import { commands } from "../../ipc/commands";
import { pendingDeepLink, setPendingDeepLink } from "../../stores/deep-link.store";
import { loadJoinedCommunity } from "../../actions/community.actions";
import { errorMessage } from "../../utils/error";

/// Asks the user before acting on an OS deep link (`rekindle://`). The
/// backend holds the link; this dialog shows only a fingerprint of the
/// key it names and sends back the user's decision. A friend invite link is
/// the Host's review instead (the intent dialog's fingerprint card, plan
/// E2.1).
const DeepLinkConsentDialog: Component = () => {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  function close(): void {
    setError(null);
    setPendingDeepLink(null);
  }

  async function dismiss(): Promise<void> {
    const request = pendingDeepLink();
    if (!request || busy()) return;
    await commands.dismissDeepLink(request.requestId).catch(() => undefined);
    close();
  }

  async function confirm(): Promise<void> {
    const request = pendingDeepLink();
    if (!request || busy()) return;
    setBusy(true);
    setError(null);
    try {
      const outcome = await commands.confirmDeepLink(request.requestId);
      close();
      if (outcome.kind === "joinedCommunity") {
        await loadJoinedCommunity(outcome.communityId);
      }
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  const fingerprint = (): string | null => {
    const request = pendingDeepLink();
    return request && "keyFingerprint" in request ? request.keyFingerprint : null;
  };

  const title = (): string => {
    switch (pendingDeepLink()?.kind) {
      case "joinCommunity":
        return "Join community?";
      default:
        return "Pairing link blocked";
    }
  };

  return (
    <Modal isOpen={pendingDeepLink() !== null} title={title()} onClose={dismiss} size="sm">
      <div class="consent-dialog">
        <Switch>
          <Match when={pendingDeepLink()?.kind === "pairingRefused"}>
            <p class="consent-dialog-text">
              A link tried to pair a new device with this account. Pairing links are only
              accepted when pasted or scanned in Settings → Devices.
            </p>
          </Match>
          <Match when={pendingDeepLink()}>
            {(request) => (
              <>
                <p class="consent-dialog-text">
                  {request().kind === "joinCommunity" &&
                    "A link wants you to join a community you have not seen before. Joining shares your community pseudonym and presence with its members."}
                </p>
                <Show when={fingerprint()}>
                  {(fp) => (
                    <p class="consent-dialog-key">
                      Key fingerprint <code>{fp()}</code>
                    </p>
                  )}
                </Show>
              </>
            )}
          </Match>
        </Switch>
        <Show when={error()}>
          <p class="consent-dialog-error" role="alert">{error()}</p>
        </Show>
        <div class="consent-dialog-actions">
          <Show
            when={pendingDeepLink()?.kind !== "pairingRefused"}
            fallback={
              <button class="sec-btn-commit" onClick={dismiss}>
                OK
              </button>
            }
          >
            <button class="sec-btn-secondary" disabled={busy()} onClick={dismiss}>
              Cancel
            </button>
            <button class="sec-btn-commit" disabled={busy()} onClick={confirm}>
              Join
            </button>
          </Show>
        </div>
      </div>
    </Modal>
  );
};

export default DeepLinkConsentDialog;
