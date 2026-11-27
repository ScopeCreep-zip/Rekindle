import { type Component, type JSX, Show, createUniqueId } from "solid-js";

interface FormFieldProps {
  label?: string;
  controlId?: string;
  error?: string | null;
  children: JSX.Element;
}

const FormField: Component<FormFieldProps> = (props) => {
  const labelId = createUniqueId();
  const groupAttributes = () => props.label && !props.controlId
    ? { role: "group" as const, "aria-labelledby": labelId }
    : {};

  return (
    <div class="form-field" {...groupAttributes()}>
      <Show when={props.label}>
        <Show
          when={props.controlId}
          fallback={<span class="form-field-label" id={labelId}>{props.label}</span>}
        >
          <label class="form-field-label" for={props.controlId}>{props.label}</label>
        </Show>
      </Show>
      {props.children}
      <Show when={props.error}>
        <div class="form-error">{props.error}</div>
      </Show>
    </div>
  );
};

export default FormField;
