import { Component, JSX, Show } from "solid-js";

interface FormFieldProps {
  label?: string;
  error?: string | null;
  children: JSX.Element;
}

const FormField: Component<FormFieldProps> = (props) => (
  <div class="form-field">
    <Show when={props.label}>
      <label class="form-field-label">{props.label}</label>
    </Show>
    {props.children}
    <Show when={props.error}>
      <div class="form-error">{props.error}</div>
    </Show>
  </div>
);

export default FormField;
