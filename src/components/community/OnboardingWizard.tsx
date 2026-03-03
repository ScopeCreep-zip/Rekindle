import { Component, For, Show, createSignal } from "solid-js";
import { commands } from "../../ipc/commands";
import type { OnboardingConfig, OnboardingAnswer } from "../../stores/types";

interface OnboardingWizardProps {
  communityId: string;
  config: OnboardingConfig;
  onComplete: () => void;
}

const OnboardingWizard: Component<OnboardingWizardProps> = (props) => {
  const [step, setStep] = createSignal(0);
  const [answers, setAnswers] = createSignal<Record<string, string[]>>({});
  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const totalSteps = () => {
    let count = 0;
    if (props.config.welcomeMessage) count++;
    count += props.config.questions.length;
    count += props.config.guideSteps.length;
    return Math.max(count, 1);
  };

  const welcomeOffset = () => (props.config.welcomeMessage ? 1 : 0);
  const questionsEnd = () => welcomeOffset() + props.config.questions.length;

  const isWelcomeStep = () => props.config.welcomeMessage && step() === 0;
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

    // Final step: submit answers
    setSubmitting(true);
    setError(null);
    try {
      const onboardingAnswers: OnboardingAnswer[] = props.config.questions.map((q) => ({
        questionId: q.questionId,
        selectedOptions: answers()[q.questionId] || [],
      }));
      await commands.submitOnboardingAnswers(props.communityId, onboardingAnswers);
      props.onComplete();
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div class="onboarding-wizard-overlay">
      <div class="onboarding-wizard">
        <div class="onboarding-wizard-header">
          <h2>Welcome</h2>
          <span class="onboarding-progress">
            {step() + 1} / {totalSteps()}
          </span>
        </div>

        <div class="onboarding-wizard-body">
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
            <button class="onboarding-btn-secondary" onClick={() => setStep(step() - 1)}>
              Back
            </button>
          </Show>
          <button
            class="onboarding-btn-primary"
            disabled={!canProceed() || submitting()}
            onClick={handleNext}
          >
            {step() < totalSteps() - 1 ? "Next" : submitting() ? "Finishing..." : "Finish"}
          </button>
        </div>
      </div>
    </div>
  );
};

export default OnboardingWizard;
