import { Component, JSX, Show } from "solid-js";

interface FormFieldProps {
  label?: string;
  /** Id on the visible label, for a control inside that takes its
   *  accessible name from it (`aria-labelledby`). */
  labelId?: string;
  error?: string | null;
  children: JSX.Element;
}

const FormField: Component<FormFieldProps> = (props) => (
  <div class="form-field">
    <Show when={props.label}>
      <label class="form-field-label" id={props.labelId}>{props.label}</label>
    </Show>
    {props.children}
    <Show when={props.error}>
      <div class="form-error">{props.error}</div>
    </Show>
  </div>
);

export default FormField;
