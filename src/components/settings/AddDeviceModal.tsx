import { Component, Show, createEffect, createSignal, onCleanup } from "solid-js";
import type { JSX } from "solid-js";

import Modal from "../common/Modal";
import FormField from "../common/FormField";
import QrScannerOverlay from "./QrScannerOverlay";
import { addToast } from "../../stores/toast.store";
import { commands } from "../../ipc/commands";

interface AddDeviceModalProps {
  isOpen: boolean;
  onClose: () => void;
  /**
   * Called after a successful pair (either side). The settings tab
   * uses this to refresh its paired-devices list without re-querying
   * here. Optional — if omitted the modal still closes itself.
   */
  onPaired?: () => void;
}

type Stage = "intro" | "show-qr" | "scan-paste" | "complete";

/**
 * Architecture §28.4 — multi-step cross-device pairing flow extracted
 * from `SettingsWindow`. Pairing is sequential (generate → wait → confirm)
 * and blocks user context, so it lives in a Modal (per Dossier 3 cognitive
 * design guidance) rather than a settings sub-tab.
 *
 * **Privacy boundary** (verbatim from architecture §28.4):
 *  - 12-word code = ~40 bits + salt; HKDF(code, salt) is the symmetric
 *    handshake key. **Not a formal PAKE** — the 5-minute TTL on
 *    `pending_pairings` is what bounds offline brute-force feasibility.
 *  - The new device generates its own master and proves knowledge of
 *    the code; the master identity is **not** exported.
 *  - The QR encodes `rekindle://pair?code=...&salt=...&route=...` —
 *    a one-time snapshot of the existing device's private route. The
 *    route rotates on its normal cadence; the captured snapshot
 *    becomes useless after the existing device's next refresh.
 */
const AddDeviceModal: Component<AddDeviceModalProps> = (props) => {
  const [stage, setStage] = createSignal<Stage>("intro");
  const [pairingSession, setPairingSession] = createSignal<{
    pairingCode: string;
    pairingSaltHex: string;
    personalRecordKey: string;
    expiresAt: number;
    existingDeviceRouteBlobHex: string;
  } | null>(null);
  const [pairingQrSvg, setPairingQrSvg] = createSignal<string | null>(null);
  const [pairingQrUri, setPairingQrUri] = createSignal<string | null>(null);
  const [generatingPairing, setGeneratingPairing] = createSignal(false);

  const [acceptCode, setAcceptCode] = createSignal("");
  const [acceptSalt, setAcceptSalt] = createSignal("");
  const [acceptRoute, setAcceptRoute] = createSignal("");
  const [acceptName, setAcceptName] = createSignal("");
  const [accepting, setAccepting] = createSignal(false);
  const [scannerOpen, setScannerOpen] = createSignal(false);

  const [acceptedDeviceName, setAcceptedDeviceName] = createSignal<string | null>(null);

  // Architecture §28.4 — 5-minute TTL countdown shown to the user so
  // they understand the pairing window is bounded. The signal ticks on
  // a 1s timer while a session is active; on expiry we drop back to
  // intro so the user has to regenerate (the backend would reject a
  // late-arriving accept anyway).
  const [nowMs, setNowMs] = createSignal(Date.now());
  let countdownHandle: ReturnType<typeof setInterval> | null = null;

  function startCountdown(): void {
    if (countdownHandle) clearInterval(countdownHandle);
    setNowMs(Date.now());
    countdownHandle = setInterval(() => setNowMs(Date.now()), 1000);
  }

  function stopCountdown(): void {
    if (countdownHandle) {
      clearInterval(countdownHandle);
      countdownHandle = null;
    }
  }

  onCleanup(stopCountdown);

  // Drop pairing state whenever the modal closes so the user starts
  // fresh on the next open. The SMPL session is one-shot anyway.
  createEffect(() => {
    if (!props.isOpen) {
      stopCountdown();
      setStage("intro");
      setPairingSession(null);
      setPairingQrSvg(null);
      setPairingQrUri(null);
      setAcceptCode("");
      setAcceptSalt("");
      setAcceptRoute("");
      setAcceptName("");
      setScannerOpen(false);
      setAcceptedDeviceName(null);
    }
  });

  // Auto-expire when the countdown reaches zero.
  createEffect(() => {
    const session = pairingSession();
    if (!session) return;
    if (nowMs() >= session.expiresAt) {
      stopCountdown();
      setPairingSession(null);
      setPairingQrSvg(null);
      setPairingQrUri(null);
      setStage("intro");
      addToast("Pairing code expired — generate a new one", "info");
    }
  });

  function remainingSeconds(): number {
    const session = pairingSession();
    if (!session) return 0;
    return Math.max(0, Math.floor((session.expiresAt - nowMs()) / 1000));
  }

  function formatCountdown(): string {
    const total = remainingSeconds();
    const minutes = Math.floor(total / 60);
    const seconds = total % 60;
    return `${minutes}:${seconds.toString().padStart(2, "0")}`;
  }

  async function generatePairing(): Promise<void> {
    setGeneratingPairing(true);
    try {
      await commands.ensurePersonalSyncRecord();
      const payload = await commands.generatePairingQrSvg();
      setPairingSession(payload.session);
      setPairingQrSvg(payload.svg);
      setPairingQrUri(payload.uri);
      setStage("show-qr");
      startCountdown();
    } catch (e) {
      console.error("Failed to start pairing session:", e);
      addToast("Failed to start pairing session", "error");
    } finally {
      setGeneratingPairing(false);
    }
  }

  // Architecture §28.4 — parse a `rekindle://pair?...` URI (pasted or
  // scanned) into the form fields.
  function applyPairingUri(uri: string): boolean {
    try {
      const url = new URL(uri.trim());
      if (url.protocol !== "rekindle:" || url.host !== "pair") return false;
      const code = url.searchParams.get("code");
      const salt = url.searchParams.get("salt");
      const route = url.searchParams.get("route");
      if (!code || !salt || !route) return false;
      setAcceptCode(code);
      setAcceptSalt(salt);
      setAcceptRoute(route);
      return true;
    } catch {
      return false;
    }
  }

  function importPairingUri(): void {
    const raw = window.prompt(
      "Paste pairing URI (rekindle://pair?code=...&salt=...&route=...) from the existing device",
    );
    if (!raw) return;
    if (!applyPairingUri(raw)) {
      addToast("That doesn't look like a valid Rekindle pairing URI.", "error");
    }
  }

  function importPairingBundle(): void {
    const raw = window.prompt("Paste pairing bundle JSON from the existing device");
    if (!raw) return;
    try {
      const parsed = JSON.parse(raw) as {
        pairingCode: string;
        pairingSaltHex: string;
        existingDeviceRouteBlobHex: string;
      };
      setAcceptCode(parsed.pairingCode);
      setAcceptSalt(parsed.pairingSaltHex);
      setAcceptRoute(parsed.existingDeviceRouteBlobHex);
    } catch (e) {
      console.error("Invalid pairing bundle JSON:", e);
      addToast("Invalid pairing bundle JSON", "error");
    }
  }

  async function copyPairingUri(): Promise<void> {
    const uri = pairingQrUri();
    if (!uri) return;
    try {
      await navigator.clipboard.writeText(uri);
      addToast("Pairing link copied", "success");
    } catch (e) {
      console.error("Failed to copy pairing URI:", e);
    }
  }

  async function acceptPairing(): Promise<void> {
    const code = acceptCode().trim();
    const salt = acceptSalt().trim();
    const route = acceptRoute().trim();
    const name = acceptName().trim();
    if (!code || !salt || !route || !name) return;
    setAccepting(true);
    try {
      await commands.acceptPairingCode(code, salt, route, name);
      setAcceptedDeviceName(name);
      setStage("complete");
      props.onPaired?.();
      addToast("Device paired", "success");
    } catch (e) {
      console.error("Failed to accept pairing:", e);
      addToast(typeof e === "string" ? e : "Pairing failed", "error");
    } finally {
      setAccepting(false);
    }
  }

  function pairingBundleJson(): string {
    const session = pairingSession();
    if (!session) return "";
    return JSON.stringify(
      {
        pairingCode: session.pairingCode,
        pairingSaltHex: session.pairingSaltHex,
        existingDeviceRouteBlobHex: session.existingDeviceRouteBlobHex,
        personalRecordKey: session.personalRecordKey,
        expiresAt: session.expiresAt,
      },
      null,
      2,
    );
  }

  function renderIntro(): JSX.Element {
    return (
      <div class="add-device-stage" aria-live="polite">
        <p class="add-device-paragraph">
          Pairing links your devices so each one can read the same
          communities and direct messages. Each device keeps its own
          keys — your master identity is never copied across the wire.
        </p>
        <p class="add-device-paragraph add-device-privacy">
          The handshake uses a 12-word code (~40 bits) + a random salt;
          HKDF(code, salt) is the symmetric key. Pairing codes expire
          after 5 minutes.
        </p>
        <div class="add-device-stage-actions">
          <button
            class="form-btn-primary"
            type="button"
            onClick={() => void generatePairing()}
            disabled={generatingPairing()}
          >
            {generatingPairing()
              ? "Generating…"
              : "I'm the existing device — show pairing code"}
          </button>
          <button
            class="form-btn-secondary"
            type="button"
            onClick={() => setStage("scan-paste")}
          >
            I'm the new device — scan or paste a code
          </button>
        </div>
      </div>
    );
  }

  function renderShowQr(): JSX.Element {
    const session = pairingSession();
    if (!session) return renderIntro();
    return (
      <div class="add-device-stage" aria-live="polite">
        <div class="add-device-banner" role="status">
          <strong>Code expires in {formatCountdown()}.</strong>{" "}
          The new device must scan or accept this code before then; the
          existing device approves automatically once the new device
          replies.
        </div>
        <Show when={pairingQrSvg()}>
          {(svg) => (
            <FormField label="Scan this QR on the new device">
              <div
                class="pairing-qr-display"
                // eslint-disable-next-line solid/no-innerhtml
                innerHTML={svg()}
                role="img"
                aria-label="Pairing QR code"
              />
              <div class="settings-hint-inline">
                Code: <strong>{session.pairingCode}</strong>
              </div>
              <div class="settings-button-row">
                <button
                  class="form-btn-secondary"
                  type="button"
                  onClick={() => void copyPairingUri()}
                >
                  Copy pairing link
                </button>
                <button
                  class="form-btn-secondary"
                  type="button"
                  onClick={() => setStage("intro")}
                >
                  Back
                </button>
              </div>
            </FormField>
          )}
        </Show>
        <FormField label="Or paste this bundle on the new device (manual fallback)">
          <textarea
            class="form-input"
            rows={8}
            readOnly
            value={pairingBundleJson()}
            onClick={(e) => e.currentTarget.select()}
            aria-label="Pairing bundle JSON for manual transfer"
          />
        </FormField>
      </div>
    );
  }

  function renderScanPaste(): JSX.Element {
    return (
      <div class="add-device-stage" aria-live="polite">
        <p class="add-device-paragraph">
          Run this on a brand-new device that needs to join your
          existing identity. Scan the QR shown on the existing device, or
          paste the link / bundle.
        </p>
        <div class="settings-button-row">
          <button
            class="form-btn-secondary"
            type="button"
            onClick={() => setScannerOpen(true)}
          >
            Scan QR code
          </button>
          <button
            class="form-btn-secondary"
            type="button"
            onClick={importPairingUri}
          >
            Paste pairing link
          </button>
          <button
            class="form-btn-secondary"
            type="button"
            onClick={importPairingBundle}
          >
            Paste bundle JSON
          </button>
        </div>
        <Show when={scannerOpen()}>
          <QrScannerOverlay
            onResult={(decoded) => {
              if (applyPairingUri(decoded)) {
                setScannerOpen(false);
              } else {
                addToast("That QR is not a Rekindle pairing code.", "error");
              }
            }}
            onClose={() => setScannerOpen(false)}
          />
        </Show>
        <FormField label="Pairing code">
          <input
            class="form-input"
            type="text"
            value={acceptCode()}
            onInput={(e) => setAcceptCode(e.currentTarget.value)}
          />
        </FormField>
        <FormField label="Pairing salt (hex)">
          <input
            class="form-input"
            type="text"
            value={acceptSalt()}
            onInput={(e) => setAcceptSalt(e.currentTarget.value)}
          />
        </FormField>
        <FormField label="Existing-device route blob (hex)">
          <input
            class="form-input"
            type="text"
            value={acceptRoute()}
            onInput={(e) => setAcceptRoute(e.currentTarget.value)}
          />
        </FormField>
        <FormField label="This device's display name">
          <input
            class="form-input"
            type="text"
            value={acceptName()}
            onInput={(e) => setAcceptName(e.currentTarget.value)}
            placeholder="e.g. Phone, Laptop"
          />
        </FormField>
        <div class="settings-button-row">
          <button
            class="form-btn-primary"
            type="button"
            onClick={() => void acceptPairing()}
            disabled={
              accepting()
              || !acceptCode()
              || !acceptSalt()
              || !acceptRoute()
              || !acceptName()
            }
          >
            {accepting() ? "Accepting…" : "Accept pairing"}
          </button>
          <button
            class="form-btn-secondary"
            type="button"
            onClick={() => setStage("intro")}
          >
            Back
          </button>
        </div>
      </div>
    );
  }

  function renderComplete(): JSX.Element {
    return (
      <div class="add-device-stage" aria-live="polite">
        <p class="add-device-paragraph">
          <strong>Device added: {acceptedDeviceName() ?? "this device"}.</strong>
        </p>
        <p class="add-device-paragraph">
          Both devices now sync read state, preferences, and onboarding
          flags through the personal SMPL record. You can manage paired
          devices from the Devices tab.
        </p>
        <button
          class="form-btn-primary"
          type="button"
          onClick={props.onClose}
        >
          Done
        </button>
      </div>
    );
  }

  function modalTitle(): string {
    switch (stage()) {
      case "show-qr": return "Add a device — show code";
      case "scan-paste": return "Add a device — scan or paste";
      case "complete": return "Device added";
      default: return "Add a device";
    }
  }

  return (
    <Modal isOpen={props.isOpen} title={modalTitle()} onClose={props.onClose} size="md">
      <Show when={stage() === "intro"}>{renderIntro()}</Show>
      <Show when={stage() === "show-qr"}>{renderShowQr()}</Show>
      <Show when={stage() === "scan-paste"}>{renderScanPaste()}</Show>
      <Show when={stage() === "complete"}>{renderComplete()}</Show>
    </Modal>
  );
};

export default AddDeviceModal;
