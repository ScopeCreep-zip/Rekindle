import { Component, Show, createSignal } from "solid-js";
import { commands } from "../../ipc/commands";

/// The quick wipe (plan E7.1): arm a duress passphrase and an optional
/// panic shortcut, or disarm both. The Host reviews each (Protected: the
/// review lists exactly what a trigger destroys and asks for the
/// identity's passphrase). Whether a duress passphrase is armed is never
/// shown: the vault header does not reveal it, so neither does this page.
const QuickWipeSection: Component = () => {
  const [duress, setDuress] = createSignal("");
  const [confirm, setConfirm] = createSignal("");
  const [shortcut, setShortcut] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);

  const mismatch = (): boolean => confirm().length > 0 && confirm() !== duress();
  const canArm = (): boolean => duress().length > 0 && confirm() === duress();

  async function arm(): Promise<void> {
    if (!canArm()) return;
    setError(null);
    setStatus(null);
    try {
      const chord = shortcut().trim();
      const state = await commands.quickWipeArm(duress(), chord.length > 0 ? chord : null);
      setDuress("");
      setConfirm("");
      setStatus(state.shortcut ? `Quick wipe armed (shortcut ${state.shortcut}).` : "Quick wipe armed.");
    } catch (e) {
      setError(`Quick wipe not armed: ${String(e)}`);
    }
  }

  async function disarm(): Promise<void> {
    setError(null);
    setStatus(null);
    try {
      await commands.quickWipeDisarm();
      setShortcut("");
      setStatus("Quick wipe disarmed.");
    } catch (e) {
      setError(`Quick wipe not disarmed: ${String(e)}`);
    }
  }

  return (
    <>
      <div class="settings-section-title">Quick Wipe</div>
      <div class="settings-hint">
        Entering the duress passphrase at any passphrase prompt, or pressing
        the panic shortcut while logged in, deletes every identity on this
        device with its messages, keys and files, the settings, logs and
        skins, and closes Rekindle. It cannot be undone. Arming again
        replaces the duress passphrase.
      </div>
      <Show when={error()}>
        <div class="form-error" role="alert">{error()}</div>
      </Show>
      <Show when={status()}>
        <div class="settings-hint" role="status">{status()}</div>
      </Show>
      <input
        class="form-input"
        type="password"
        autocomplete="off"
        aria-label="Duress passphrase"
        placeholder="Duress passphrase"
        value={duress()}
        onInput={(e) => setDuress(e.currentTarget.value)}
      />
      <input
        class="form-input"
        type="password"
        autocomplete="off"
        aria-label="Confirm duress passphrase"
        placeholder="Confirm duress passphrase"
        value={confirm()}
        onInput={(e) => setConfirm(e.currentTarget.value)}
      />
      <Show when={mismatch()}>
        <div class="form-error">The passphrases do not match.</div>
      </Show>
      <input
        class="form-input"
        type="text"
        autocomplete="off"
        aria-label="Panic shortcut"
        placeholder="Panic shortcut (optional), e.g. CommandOrControl+Shift+F12"
        value={shortcut()}
        onInput={(e) => setShortcut(e.currentTarget.value)}
      />
      <div class="form-field-row">
        <button class="form-btn-secondary" disabled={!canArm()} onClick={() => void arm()}>
          Arm Quick Wipe
        </button>
        <button class="form-btn-secondary" onClick={() => void disarm()}>
          Disarm Quick Wipe
        </button>
      </div>
    </>
  );
};

export default QuickWipeSection;
