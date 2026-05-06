import { Component, For, Show, createMemo, createSignal } from "solid-js";

import Modal from "../common/Modal";
import LoadingButton from "../common/LoadingButton";
import { handleSubmitOnboarding } from "../../handlers/community.handlers";
import type { OnboardingConfig, OnboardingAnswer } from "../../stores/types";

interface OnboardingWizardProps {
  communityId: string;
  config: OnboardingConfig;
  onComplete: () => void;
  /**
   * Optional escape hatch. For non-gated communities, parent passes a
   * handler that records "skipped" — backend treats empty answers as
   * default-everyone-role. For gated (`config.mode === "gated"`)
   * communities the parent passes nothing and Modal disables Esc + X
   * entirely so the user can't bypass mandatory rules acknowledgement.
   */
  onCancel?: () => void;
}

const OnboardingWizard: Component<OnboardingWizardProps> = (props) => {
  const [step, setStep] = createSignal(0);
  const [answers, setAnswers] = createSignal<Record<string, string[]>>({});
  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  // Architecture §19.2 step 3 — gated mode requires explicit local
  // acknowledgment of the community rules before continuing.
  const [rulesAcknowledged, setRulesAcknowledged] = createSignal(false);

  const isGated = createMemo(() => props.config.mode === "gated");
  const showRulesStep = createMemo(
    () => isGated() && Boolean(props.config.welcomeMessage),
  );

  const totalSteps = () => {
    let count = 0;
    if (showRulesStep()) count++;
    else if (props.config.welcomeMessage) count++;
    count += props.config.questions.length;
    count += props.config.guideSteps.length;
    return Math.max(count, 1);
  };

  const welcomeOffset = () =>
    showRulesStep() || props.config.welcomeMessage ? 1 : 0;
  const questionsEnd = () => welcomeOffset() + props.config.questions.length;

  const isWelcomeStep = () =>
    !showRulesStep() && props.config.welcomeMessage && step() === 0;
  const isRulesStep = () => showRulesStep() && step() === 0;
  const isQuestionStep = () => {
    const s = step();
    return s >= welcomeOffset() && s < questionsEnd();
  };
  const isGuideStep = () => step() >= questionsEnd();

  const currentQuestion = () => {
    const idx = step() - welcomeOffset();
    return props.config.questions[idx];
  };

  const currentGuideStep = () => {
    const idx = step() - questionsEnd();
    return props.config.guideSteps[idx];
  };

  const toggleOption = (questionId: string, optionId: string, singleSelect: boolean) => {
    setAnswers((prev) => {
      const current = prev[questionId] || [];
      if (singleSelect) {
        return { ...prev, [questionId]: [optionId] };
      }
      if (current.includes(optionId)) {
        return { ...prev, [questionId]: current.filter((id) => id !== optionId) };
      }
      return { ...prev, [questionId]: [...current, optionId] };
    });
  };

  const canProceed = () => {
    if (isRulesStep()) return rulesAcknowledged();
    if (isWelcomeStep()) return true;
    if (isQuestionStep()) {
      const q = currentQuestion();
      if (!q) return true;
      if (q.required) {
        const selected = answers()[q.questionId] || [];
        return selected.length > 0;
      }
      return true;
    }
    return true;
  };

  const handleNext = async () => {
    if (step() < totalSteps() - 1) {
      setStep(step() + 1);
      return;
    }

    // Final step: submit answers (gated communities require ack).
    setSubmitting(true);
    setError(null);
    try {
      const onboardingAnswers: OnboardingAnswer[] = props.config.questions.map((q) => ({
        questionId: q.questionId,
        selectedOptions: answers()[q.questionId] || [],
      }));
      const ack = isGated() ? rulesAcknowledged() : undefined;
      const success = await handleSubmitOnboarding(
        props.communityId,
        onboardingAnswers,
        ack,
      );
      if (success) {
        props.onComplete();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmitting(false);
    }
  };

  // Gated communities require completion — no escape hatch. Non-gated
  // communities expose Esc + close button via the optional `onCancel`.
  const dismissable = createMemo(() => !isGated() && Boolean(props.onCancel));

  return (
    <Modal
      isOpen={true}
      title={isGated() ? "Welcome — please review" : "Welcome"}
      onClose={props.onCancel ?? (() => {})}
      dismissable={dismissable()}
      size="lg"
    >
      <div class="onboarding-wizard">
        <Show when={isGated()}>
          <div class="onboarding-wizard-gated-banner" role="status">
            This community requires onboarding before you can participate.
            Complete the steps below to continue.
          </div>
        </Show>
        <div class="onboarding-wizard-header">
          <span class="onboarding-progress">
            Step {step() + 1} of {totalSteps()}
          </span>
        </div>

        <div class="onboarding-wizard-body">
          <Show when={isRulesStep()}>
            <div class="onboarding-rules">
              <h3>Community rules</h3>
              <pre class="onboarding-rules-text">{props.config.welcomeMessage}</pre>
              <label class="onboarding-rules-ack">
                <input
                  type="checkbox"
                  checked={rulesAcknowledged()}
                  onChange={(e) => setRulesAcknowledged(e.currentTarget.checked)}
                />
                <span>I have read and agree to the community rules.</span>
              </label>
            </div>
          </Show>

          <Show when={isWelcomeStep()}>
            <div class="onboarding-welcome">
              <p>{props.config.welcomeMessage}</p>
            </div>
          </Show>

          <Show when={isQuestionStep() && currentQuestion()}>
            {(q) => (
              <div class="onboarding-question">
                <h3>{q().title}</h3>
                <Show when={q().description}>
                  <p class="onboarding-question-desc">{q().description}</p>
                </Show>
                <div class="onboarding-options">
                  <For each={q().options}>
                    {(opt) => {
                      const selected = () =>
                        (answers()[q().questionId] || []).includes(opt.optionId);
                      return (
                        <button
                          class="onboarding-option"
                          classList={{ selected: selected() }}
                          onClick={() =>
                            toggleOption(q().questionId, opt.optionId, q().singleSelect)
                          }
                        >
                          <span class="onboarding-option-title">{opt.title}</span>
                          <Show when={opt.description}>
                            <span class="onboarding-option-desc">{opt.description}</span>
                          </Show>
                        </button>
                      );
                    }}
                  </For>
                </div>
              </div>
            )}
          </Show>

          <Show when={isGuideStep() && currentGuideStep()}>
            {(gs) => (
              <div class="onboarding-guide-step">
                <Show when={gs().emoji}>
                  <span class="onboarding-guide-emoji">{gs().emoji}</span>
                </Show>
                <h3>{gs().title}</h3>
                <p>{gs().description}</p>
              </div>
            )}
          </Show>
        </div>

        <Show when={error()}>
          <div class="onboarding-error">{error()}</div>
        </Show>

        <div class="onboarding-wizard-footer">
          <Show when={step() > 0}>
            <button class="form-btn-secondary" onClick={() => setStep(step() - 1)}>
              Back
            </button>
          </Show>
          <LoadingButton
            loading={step() === totalSteps() - 1 && submitting()}
            disabled={!canProceed()}
            onClick={() => void handleNext()}
            loadingLabel="Finishing"
          >
            {step() < totalSteps() - 1 ? "Next" : "Finish"}
          </LoadingButton>
        </div>
      </div>
    </Modal>
  );
};

export default OnboardingWizard;
