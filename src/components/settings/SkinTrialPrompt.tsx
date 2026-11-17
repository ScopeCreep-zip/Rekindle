import { Component } from "solid-js";
import type { SkinTrial } from "../../ipc/commands/dto";

/**
 * "Keep this skin?" (plan E7.7.2): the Host's trial of a library skin,
 * drawn on the reserved `.sec-review` card so the skin on trial cannot
 * restyle or hide the way back. The Host reverts when the trial runs out;
 * this only shows the time left and sends the answer.
 */
const SkinTrialPrompt: Component<{
  trial: SkinTrial;
  secondsLeft: number;
  onAnswer: (keep: boolean) => void;
}> = (props) => (
  <div class="skin-trial">
    <section class="sec-review" aria-labelledby="skin-trial-title">
      <div class="sec-review-subject" id="skin-trial-title">
        Keep this skin?
      </div>
      <p class="sec-review-fact">
        {props.trial.name} goes back to the previous skin in {props.secondsLeft} s unless you keep
        it.
      </p>
    </section>
    <div class="skin-trial-actions">
      <button class="sec-btn-commit" onClick={() => props.onAnswer(true)}>
        Keep this skin
      </button>
      <button class="sec-btn-secondary" onClick={() => props.onAnswer(false)}>
        Revert
      </button>
    </div>
  </div>
);

export default SkinTrialPrompt;
