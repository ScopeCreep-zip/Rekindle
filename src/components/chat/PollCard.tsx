import { Component, For, Show, createEffect, createSignal } from "solid-js";
import type { MessagePoll } from "../../stores/chat.store";

interface PollCardProps {
  poll: MessagePoll;
  onVote?: (selectedAnswers: number[]) => void;
  onClose?: () => void;
}

const PollCard: Component<PollCardProps> = (props) => {
  const [selectedAnswers, setSelectedAnswers] = createSignal<number[]>([]);

  createEffect(() => {
    setSelectedAnswers(props.poll.selectedAnswers ?? []);
  });

  function toggleAnswer(index: number): void {
    if (props.poll.closed) return;
    if (!props.poll.multiSelect) {
      props.onVote?.([index]);
      return;
    }
    setSelectedAnswers((prev) =>
      prev.includes(index) ? prev.filter((value) => value !== index) : [...prev, index].sort((a, b) => a - b),
    );
  }

  function submitVote(): void {
    const selected = selectedAnswers();
    if (selected.length === 0) return;
    props.onVote?.(selected);
  }

  function isSelected(index: number): boolean {
    return selectedAnswers().includes(index);
  }

  return (
    <div class="poll-card">
      <div class="poll-card-header">
        <div class="poll-card-question">{props.poll.question}</div>
        <Show when={props.poll.closed}>
          <span class="poll-card-status">Closed</span>
        </Show>
      </div>
      <div class="poll-card-meta">
        <span>{props.poll.multiSelect ? "Multiple choice" : "Single choice"}</span>
        <Show when={props.poll.expiresAt}>
          <span>Ends at {new Date(props.poll.expiresAt!).toLocaleString()}</span>
        </Show>
      </div>
      <div class="poll-card-answers">
        <For each={props.poll.answers}>
          {(answer) => (
            <button
              class={`poll-card-answer ${isSelected(answer.index) ? "poll-card-answer-selected" : ""}`}
              disabled={props.poll.closed}
              onClick={() => toggleAnswer(answer.index)}
              title={answer.voters.join(", ")}
            >
              <span class="poll-card-answer-text">{answer.text}</span>
              <span class="poll-card-answer-count">{answer.voteCount}</span>
            </button>
          )}
        </For>
      </div>
      <Show when={!props.poll.closed}>
        <div class="poll-card-actions">
          <Show when={props.poll.multiSelect}>
            <button
              class="poll-card-submit"
              disabled={selectedAnswers().length === 0}
              onClick={submitVote}
            >
              Vote
            </button>
          </Show>
          <Show when={props.onClose}>
            <button class="poll-card-close" onClick={() => props.onClose?.()}>
              Close poll
            </button>
          </Show>
        </div>
      </Show>
    </div>
  );
};

export default PollCard;
