import { type Component, createMemo, createSignal, For, Show } from "solid-js";
import { handleSetDisappearingTimer } from "../../actions/disappearing.actions";
import { disappearingState } from "../../stores/disappearing.store";
import { CUSTOM_UNITS, durationText } from "../../utils/disappearing";
import SkinGlyph from "../common/SkinGlyph";

/** The chat window's disappearing-messages control (plan E7.1): a header
 *  button showing the timer in force, opening the Host's presets and a
 *  custom value up to the Host's bound. Choosing one asks the Host, which
 *  reviews it (`SetDisappearingTimer`); the button follows the
 *  `disappearingTimerChanged` the Host then publishes. */
const DisappearingTimerControl: Component<{ conversation: string; label: string }> = (props) => {
  const view = createMemo(() => disappearingState.timers[props.conversation]);
  const seconds = createMemo(() => view()?.seconds ?? 0);
  const [open, setOpen] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [amount, setAmount] = createSignal("1");
  const [unit, setUnit] = createSignal(CUSTOM_UNITS[2].seconds);
  const panelId = `disappearing-${props.conversation}`;

  const custom = createMemo(() => {
    const n = Number(amount());
    return Number.isInteger(n) && n > 0 ? n * unit() : null;
  });
  const customFits = createMemo(() => {
    const c = custom();
    const max = view()?.maxSeconds;
    return c !== null && max !== undefined && c <= max;
  });

  async function choose(value: number): Promise<void> {
    setError(null);
    const err = await handleSetDisappearingTimer(props.conversation, value);
    if (err) setError(err);
    else setOpen(false);
  }

  return (
    <Show when={view()}>
      {(v) => (
        <div class="chat-disappearing">
          <button
            type="button"
            class="chat-disappearing-btn"
            classList={{ "chat-disappearing-btn-on": seconds() > 0 }}
            onClick={() => setOpen(!open())}
            title={`Disappearing messages: ${durationText(seconds())}`}
            aria-label={`Disappearing messages in ${props.label}: ${durationText(seconds())}`}
            aria-expanded={open()}
            aria-controls={open() ? panelId : undefined}
          >
            <SkinGlyph name="timer-sand" />
            <span class="chat-disappearing-value">{durationText(seconds())}</span>
          </button>
          <Show when={open()}>
            <div
              id={panelId}
              class="chat-disappearing-panel"
              role="group"
              aria-label="Disappearing messages"
              onKeyDown={(e) => {
                if (e.key === "Escape") setOpen(false);
              }}
            >
              <p class="chat-disappearing-help">
                New messages disappear this long after they are sent, or after they are read for the
                person receiving them. Copies already saved, or kept by a modified app, are not
                removed.
              </p>
              <div class="chat-disappearing-presets" role="radiogroup" aria-label="Timer">
                <For each={v().presets}>
                  {(preset) => (
                    <button
                      type="button"
                      class="chat-disappearing-preset"
                      role="radio"
                      aria-checked={preset === seconds()}
                      onClick={() => void choose(preset)}
                    >
                      {durationText(preset)}
                    </button>
                  )}
                </For>
              </div>
              <div class="form-field-row">
                <label class="chat-disappearing-custom-label" for={`${panelId}-amount`}>
                  Custom
                </label>
                <input
                  id={`${panelId}-amount`}
                  class="chat-disappearing-amount"
                  type="number"
                  min="1"
                  step="1"
                  value={amount()}
                  onInput={(e) => setAmount(e.currentTarget.value)}
                />
                <select
                  class="chat-disappearing-unit"
                  aria-label="Unit"
                  value={unit()}
                  onChange={(e) => setUnit(Number(e.currentTarget.value))}
                >
                  <For each={CUSTOM_UNITS}>
                    {(u) => <option value={u.seconds}>{u.label}</option>}
                  </For>
                </select>
                <button
                  type="button"
                  class="chat-disappearing-set"
                  disabled={!customFits()}
                  onClick={() => void choose(custom()!)}
                >
                  Set
                </button>
              </div>
              <Show when={custom() !== null && !customFits()}>
                <p class="form-error" role="alert">
                  The longest timer is {durationText(v().maxSeconds)}.
                </p>
              </Show>
              <Show when={error()}>
                <p class="form-error" role="alert">
                  {error()}
                </p>
              </Show>
            </div>
          </Show>
        </div>
      )}
    </Show>
  );
};

export default DisappearingTimerControl;
