import { type Component, createSignal, onCleanup, onMount, Show } from "solid-js";
import { subscribeQuickWipe } from "../../ipc/channels";
import { commands } from "../../ipc/commands";
import { errorMessage } from "../../utils/error";

/// The quick wipe (plan E7.1): arm a duress passphrase and an optional
/// panic shortcut, or disarm both. The Host reviews each (Protected: the
/// review lists exactly what a trigger destroys and asks for the
/// identity's passphrase). The status is the Host's `QuickWipeChanged`,
/// published when the review commits. Whether a duress passphrase is armed
/// is never read back: the vault header does not reveal it, so this page
/// shows only what this session committed.
const QuickWipeSection: Component = () => {
  const [duress, setDuress] = createSignal("");
  const [confirm, setConfirm] = createSignal("");
  const [shortcut, setShortcut] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);
  // The last arming carried a chord and was refused: nothing was armed, and
  // arming the duress passphrase alone is offered as its own action, which
  // goes through the Host's review like any arming.
  const [offerAlone, setOfferAlone] = createSignal(false);

  const mismatch = (): boolean => confirm().length > 0 && confirm() !== duress();
  const canArm = (): boolean => duress().length > 0 && confirm() === duress();

  onMount(() => {
    const sub = subscribeQuickWipe((change) => {
      if (!change.armed) setStatus("Quick wipe disarmed.");
      else if (change.shortcut) setStatus(`Quick wipe armed (shortcut ${change.shortcut}).`);
      else setStatus("Quick wipe armed.");
    });
    onCleanup(() => void sub.then((unlisten) => unlisten()));
  });

  async function arm(chord: string | null): Promise<void> {
    if (!canArm()) return;
    setError(null);
    setStatus(null);
    setOfferAlone(false);
    try {
      await commands.quickWipeArm(duress(), chord);
      setDuress("");
      setConfirm("");
      if (chord === null) setShortcut("");
    } catch (e) {
      setError(`Quick wipe not armed: ${errorMessage(e)}`);
      // A refusal, not a cancelled review: the chord was what failed.
      const refused =
        typeof e === "object" && e !== null && (e as { type?: unknown }).type === "failed";
      setOfferAlone(chord !== null && refused);
    }
  }

  function chordEntered(): string | null {
    const chord = shortcut().trim();
    return chord.length > 0 ? chord : null;
  }

  async function disarm(): Promise<void> {
    setError(null);
    setStatus(null);
    setOfferAlone(false);
    try {
      await commands.quickWipeDisarm();
      setShortcut("");
    } catch (e) {
      setError(`Quick wipe not disarmed: ${errorMessage(e)}`);
    }
  }

  return (
    <>
      <div class="settings-section-title">Quick Wipe</div>
      <div class="settings-hint">
        Entering the duress passphrase at any passphrase prompt, or pressing the panic shortcut
        while logged in, deletes every identity on this device with its messages, keys and files,
        the settings, logs and skins, and closes Rekindle. It cannot be undone. Arming again
        replaces the duress passphrase.
      </div>
      <Show when={error()}>
        <div class="form-error" role="alert">
          {error()}
        </div>
      </Show>
      <Show when={status()}>
        <div class="settings-hint" role="status">
          {status()}
        </div>
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
        <button
          type="button"
          class="form-btn-secondary"
          disabled={!canArm()}
          onClick={() => void arm(chordEntered())}
        >
          Arm Quick Wipe
        </button>
        <Show when={offerAlone()}>
          <button
            type="button"
            class="form-btn-secondary"
            disabled={!canArm()}
            onClick={() => void arm(null)}
          >
            Arm Without Shortcut
          </button>
        </Show>
        <button type="button" class="form-btn-secondary" onClick={() => void disarm()}>
          Disarm Quick Wipe
        </button>
      </div>
    </>
  );
};

export default QuickWipeSection;
