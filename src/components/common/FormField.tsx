import { type Component, createUniqueId, type JSX, Show } from "solid-js";

interface FormFieldProps {
  label?: string;
  controlId?: string;
  labelId?: string;
  error?: string | null;
  children: JSX.Element;
}

const FormField: Component<FormFieldProps> = (props) => {
  const generatedId = createUniqueId();
  const labelId = () => props.labelId ?? generatedId;
  const groupAttributes = () =>
    props.label && !props.controlId && !props.labelId
      ? { role: "group" as const, "aria-labelledby": labelId() }
      : {};

  return (
    <div class="form-field" {...groupAttributes()}>
      <Show when={props.label}>
        <Show
          when={props.controlId}
          fallback={
            <span class="form-field-label" id={labelId()}>
              {props.label}
            </span>
          }
        >
          <label class="form-field-label" id={props.labelId} for={props.controlId}>
            {props.label}
          </label>
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
