import { Component, For, createEffect, createSignal } from "solid-js";
import Modal from "../common/Modal";
import { handleCreatePoll } from "../../handlers/community.handlers";

interface CreatePollModalProps {
  isOpen: boolean;
  communityId: string;
  channelId: string;
  messageId: string;
  onClose: () => void;
}

const CreatePollModal: Component<CreatePollModalProps> = (props) => {
  const [question, setQuestion] = createSignal("");
  const [answers, setAnswers] = createSignal(["", ""]);
  const [multiSelect, setMultiSelect] = createSignal(false);
  const [durationMinutes, setDurationMinutes] = createSignal("");
  const [validationError, setValidationError] = createSignal("");

  createEffect(() => {
    if (props.isOpen) {
      setQuestion("");
      setAnswers(["", ""]);
      setMultiSelect(false);
      setDurationMinutes("");
      setValidationError("");
    }
  });

  function updateAnswer(index: number, value: string): void {
    setAnswers((prev) => prev.map((answer, answerIndex) => (answerIndex === index ? value : answer)));
  }

  function addAnswer(): void {
    setAnswers((prev) => (prev.length >= 10 ? prev : [...prev, ""]));
  }

  function removeAnswer(index: number): void {
    setAnswers((prev) => (prev.length <= 2 ? prev : prev.filter((_, answerIndex) => answerIndex !== index)));
  }

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const trimmedQuestion = question().trim();
    const trimmedAnswers = answers().map((answer) => answer.trim()).filter(Boolean);
    if (!trimmedQuestion) {
      setValidationError("Question is required");
      return;
    }
    if (trimmedAnswers.length < 2) {
      setValidationError("At least two answers are required");
      return;
    }
    const trimmedDuration = durationMinutes().trim();
    let durationSeconds: number | undefined;
    if (trimmedDuration) {
      const minutes = Number(trimmedDuration);
      if (!Number.isFinite(minutes) || minutes <= 0) {
        setValidationError("Duration must be a positive number of minutes");
        return;
      }
      durationSeconds = Math.floor(minutes * 60);
    }
    setValidationError("");
    const pollId = await handleCreatePoll(
      props.communityId,
      props.channelId,
      props.messageId,
      trimmedQuestion,
      trimmedAnswers,
      multiSelect(),
      durationSeconds,
    );
    if (pollId) {
      props.onClose();
    }
  }

  return (
    <Modal isOpen={props.isOpen} title="Create Poll" onClose={props.onClose}>
      <form class="form-group" onSubmit={handleSubmit}>
        <input
          class="form-input"
          type="text"
          placeholder="Poll question..."
          value={question()}
          onInput={(e) => setQuestion(e.currentTarget.value)}
        />
        <For each={answers()}>
          {(answer, index) => (
            <div class="form-row poll-create-answer-row">
              <input
                class="form-input"
                type="text"
                placeholder={`Answer ${index() + 1}`}
                value={answer}
                onInput={(e) => updateAnswer(index(), e.currentTarget.value)}
              />
              <button
                class="poll-create-remove-btn"
                type="button"
                disabled={answers().length <= 2}
                onClick={() => removeAnswer(index())}
              >
                Remove
              </button>
            </div>
          )}
        </For>
        <button
          class="poll-create-add-btn"
          type="button"
          disabled={answers().length >= 10}
          onClick={addAnswer}
        >
          Add answer
        </button>
        <label class="poll-create-checkbox">
          <input
            type="checkbox"
            checked={multiSelect()}
            onChange={(e) => setMultiSelect(e.currentTarget.checked)}
          />
          Allow multiple answers
        </label>
        <input
          class="form-input"
          type="number"
          min="1"
          step="1"
          placeholder="Duration in minutes (optional)"
          value={durationMinutes()}
          onInput={(e) => setDurationMinutes(e.currentTarget.value)}
        />
        {validationError() && <div class="form-error">{validationError()}</div>}
        <button class="form-btn-primary" type="submit">
          Create Poll
        </button>
      </form>
    </Modal>
  );
};

export default CreatePollModal;
