import { Component, createEffect, createSignal, For, Match, Show, Switch } from "solid-js";
import { Dialog } from "@kobalte/core/dialog";
import { cancelReview, commitReview, intentState } from "../../stores/intent.store";
import {
  commitLabel, escalationText, factText, kindPhrase, notifiesText, reversibilityText,
  subjectLabel, typedName,
} from "../../utils/intent_text";

/**
 * The review of an action the Host holds (ADR 0019, plan U1.6): the facts
 * it states and the one proof its class asks for. Commit is a verb + object
 * button with no default; Acknowledge a checkbox that states the fact;
 * TypeSubject a text field for the object's own name (paste allowed);
 * Passphrase a password field (paste allowed). Only the final button of a
 * Destructive or Protected review carries the warning style. Kobalte's
 * Dialog traps focus and returns it to the opener; Esc cancels.
 *
 * A security surface (plan E7.7.0): its buttons are the reserved
 * `.sec-btn-*` controls, which no skin can restyle. The destructive commit
 * is its own button, never the form's default: Return in the proof field
 * does not press it (D0; Apple HIG "Don't assign the primary role to a
 * button that performs a destructive action"). Its verb-and-object label is
 * the non-colour cue.
 */
const IntentDialog: Component = () => {
  const [acknowledged, setAcknowledged] = createSignal(false);
  const [typed, setTyped] = createSignal("");
  const [passphrase, setPassphrase] = createSignal("");

  // A new review starts with an empty proof.
  createEffect(() => {
    void intentState.review?.intentId;
    setAcknowledged(false);
    setTyped("");
    setPassphrase("");
  });

  const review = () => intentState.review;
  const warning = () => {
    const r = review();
    return r !== null && (r.class === "destructive" || r.class === "protected");
  };
  const ready = (): boolean => {
    const r = review();
    if (r === null || intentState.busy) return false;
    switch (r.proof) {
      case "acknowledge":
        return acknowledged();
      case "typeSubject":
        return typed() === typedName(r.subject);
      case "passphrase":
        return passphrase().length > 0;
      default:
        return true;
    }
  };

  // Return submits the form; it commits only a review that is not a warning.
  function submit(e: Event): void {
    e.preventDefault();
    if (!warning()) commit();
  }

  function commit(): void {
    const r = review();
    if (r === null || !ready()) return;
    switch (r.proof) {
      case "acknowledge":
        void commitReview("acknowledged");
        return;
      case "typeSubject":
        void commitReview({ typed: typed() });
        return;
      case "passphrase":
        void commitReview({ passphrase: passphrase() });
        return;
      default:
        void commitReview("commit");
    }
  }

  return (
    <Dialog
      open={review() !== null}
      onOpenChange={(open) => {
        if (!open) void cancelReview();
      }}
      modal={true}
    >
      <Dialog.Portal>
        <Dialog.Overlay class="modal-overlay" />
        <div class="modal-overlay-positioner">
          <Show when={review()}>
            {(r) => (
              <Dialog.Content class="modal-container modal-container-sm intent-dialog">
                <div class="modal-header">
                  <Dialog.Title class="modal-title">{kindPhrase(r().kind)}</Dialog.Title>
                </div>
                <form class="modal-body intent-dialog-body" onSubmit={submit}>
                  <div class="intent-dialog-subject">{subjectLabel(r().subject)}</div>
                  <Show when={r().facts.length > 0 || r().escalations.length > 0}>
                    <ul class="intent-dialog-facts">
                      <For each={r().escalations}>
                        {(escalation) => (
                          <li class="intent-dialog-escalation">{escalationText(escalation)}</li>
                        )}
                      </For>
                      <For each={r().facts}>{(fact) => <li>{factText(fact)}</li>}</For>
                    </ul>
                  </Show>
                  <div class="intent-dialog-meta">
                    {reversibilityText(r().reversibility)}
                    <Show when={notifiesText(r().notifies)}>{(text) => <> {text()}</>}</Show>
                  </div>
                  <Switch>
                    <Match when={r().proof === "acknowledge"}>
                      <label class="intent-dialog-acknowledge">
                        <input
                          type="checkbox"
                          checked={acknowledged()}
                          onChange={(e) => setAcknowledged(e.currentTarget.checked)}
                        />
                        <span>{r().facts.length > 0 ? factText(r().facts[0]) : "I understand."}</span>
                      </label>
                    </Match>
                    <Match when={r().proof === "typeSubject"}>
                      <label class="intent-dialog-field">
                        <span>Type {typedName(r().subject)} to confirm</span>
                        <input
                          class="form-input"
                          type="text"
                          autocomplete="off"
                          value={typed()}
                          onInput={(e) => setTyped(e.currentTarget.value)}
                        />
                      </label>
                    </Match>
                    <Match when={r().proof === "passphrase"}>
                      <label class="intent-dialog-field">
                        <span>Passphrase for {subjectLabel(r().subject)}</span>
                        <input
                          class="form-input"
                          type="password"
                          autocomplete="current-password"
                          value={passphrase()}
                          onInput={(e) => setPassphrase(e.currentTarget.value)}
                        />
                      </label>
                    </Match>
                  </Switch>
                  <Show when={intentState.error !== null}>
                    <div class="form-error" role="alert">{intentState.error}</div>
                  </Show>
                  <div class="intent-dialog-actions">
                    <button type="button" class="sec-btn-secondary" onClick={() => void cancelReview()}>
                      Cancel
                    </button>
                    <Show
                      when={warning()}
                      fallback={
                        <button type="submit" class="sec-btn-commit" disabled={!ready()}>
                          {commitLabel(r())}
                        </button>
                      }
                    >
                      <button
                        type="button"
                        class="sec-btn-commit-danger"
                        data-role="destructive"
                        disabled={!ready()}
                        onClick={commit}
                      >
                        {commitLabel(r())}
                      </button>
                    </Show>
                  </div>
                </form>
              </Dialog.Content>
            )}
          </Show>
        </div>
      </Dialog.Portal>
    </Dialog>
  );
};

export default IntentDialog;
