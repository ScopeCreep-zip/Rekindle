import { Component, createSignal, createEffect } from "solid-js";
import Modal from "../common/Modal";

// Architecture §32 Phase 6 W19 line 4065 — auto-archive timeout choices.
const AUTO_ARCHIVE_OPTIONS: { value: number; label: string }[] = [
  { value: 3600, label: "1 hour of inactivity" },
  { value: 86400, label: "24 hours of inactivity" },
  { value: 259200, label: "3 days of inactivity" },
  { value: 604800, label: "1 week of inactivity" },
];

interface CreateThreadModalProps {
  isOpen: boolean;
  initialName: string;
  onClose: () => void;
  onSubmit: (name: string, autoArchiveSeconds: number) => Promise<void> | void;
}

const CreateThreadModal: Component<CreateThreadModalProps> = (props) => {
  const [name, setName] = createSignal("");
  const [autoArchive, setAutoArchive] = createSignal<number>(86400);

  createEffect(() => {
    if (props.isOpen) {
      setName(props.initialName);
      setAutoArchive(86400);
    }
  });

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const value = name().trim();
    if (!value) return;
    await props.onSubmit(value, autoArchive());
  }

  return (
    <Modal isOpen={props.isOpen} title="Create Thread" onClose={props.onClose}>
      <form class="form-group" onSubmit={(e) => void handleSubmit(e)}>
        <input
          class="form-input"
          type="text"
          placeholder="Thread name..."
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
        />
        <select
          class="form-select"
          value={autoArchive()}
          onChange={(e) => setAutoArchive(parseInt(e.currentTarget.value, 10) || 86400)}
        >
          {AUTO_ARCHIVE_OPTIONS.map((opt) => (
            <option value={opt.value}>{opt.label}</option>
          ))}
        </select>
        <button class="form-btn-primary" type="submit" disabled={name().trim().length === 0}>
          Create
        </button>
      </form>
    </Modal>
  );
};

export default CreateThreadModal;
