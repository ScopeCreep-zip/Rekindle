import { Component, Show, createSignal, createEffect } from "solid-js";
import Modal from "./Modal";
import LoadingButton from "./LoadingButton";

interface SimpleInputModalProps {
  isOpen: boolean;
  title: string;
  onClose: () => void;
  onSubmit: (value: string, secondaryValue?: string) => Promise<void>;
  placeholder?: string;
  submitLabel?: string;
  initialValue?: string;
  validate?: (value: string) => string | null;
  secondaryPlaceholder?: string;
  secondaryInitialValue?: string;
}

const SimpleInputModal: Component<SimpleInputModalProps> = (props) => {
  const [value, setValue] = createSignal("");
  const [secondary, setSecondary] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [submitting, setSubmitting] = createSignal(false);

  createEffect(() => {
    if (props.isOpen) {
      setValue(props.initialValue ?? "");
      setSecondary(props.secondaryInitialValue ?? "");
      setError(null);
      setSubmitting(false);
    }
  });

  async function handleSubmit(e: Event): Promise<void> {
    e.preventDefault();
    const v = value().trim();
    if (!v) return;
    if (props.validate) {
      const err = props.validate(v);
      if (err) { setError(err); return; }
    }
    setError(null);
    setSubmitting(true);
    try {
      await props.onSubmit(v, props.secondaryPlaceholder ? secondary().trim() : undefined);
      props.onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Modal isOpen={props.isOpen} title={props.title} onClose={props.onClose}>
      <form class="form-group" onSubmit={handleSubmit}>
        <input
          class="form-input"
          type="text"
          placeholder={props.placeholder ?? ""}
          value={value()}
          onInput={(e) => setValue(e.currentTarget.value)}
        />
        <Show when={props.secondaryPlaceholder}>
          <input
            class="form-input"
            type="text"
            placeholder={props.secondaryPlaceholder}
            value={secondary()}
            onInput={(e) => setSecondary(e.currentTarget.value)}
          />
        </Show>
        <Show when={error()}>
          <div class="form-error">{error()}</div>
        </Show>
        <LoadingButton
          type="submit"
          loading={submitting()}
          disabled={!value().trim()}
          loadingLabel="Submitting"
        >
          {props.submitLabel ?? "Submit"}
        </LoadingButton>
      </form>
    </Modal>
  );
};

export default SimpleInputModal;
