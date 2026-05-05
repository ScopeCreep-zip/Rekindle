import { Component, JSX, Show } from "solid-js";

type Variant = "primary" | "secondary" | "danger";

interface LoadingButtonProps {
  loading: boolean;
  disabled?: boolean;
  variant?: Variant;
  type?: "submit" | "button" | "reset";
  onClick?: (e: MouseEvent) => void;
  loadingLabel?: string;
  ariaLabel?: string;
  children: JSX.Element;
  class?: string;
}

const VARIANT_CLASS: Record<Variant, string> = {
  primary: "form-btn-primary",
  secondary: "form-btn-secondary",
  danger: "form-btn-danger",
};

/**
 * Architecture §32 a11y — wraps the canonical async-submit button so
 * every busy state announces `aria-busy` and disables click while the
 * promise is in flight. Replaces ~12 ad-hoc `disabled={submitting()}`
 * patterns scattered across the modals and forms.
 */
const LoadingButton: Component<LoadingButtonProps> = (props) => {
  const variantClass = () => VARIANT_CLASS[props.variant ?? "primary"];
  const className = () =>
    [variantClass(), props.class].filter(Boolean).join(" ");

  return (
    <button
      type={props.type ?? "button"}
      class={className()}
      disabled={props.loading || props.disabled}
      aria-busy={props.loading}
      aria-label={props.ariaLabel}
      onClick={props.onClick}
    >
      <Show when={props.loading} fallback={props.children}>
        <span class="loading-button-spinner" aria-hidden="true">…</span>
        <span class="loading-button-label">
          {props.loadingLabel ?? "Working"}
        </span>
      </Show>
    </button>
  );
};

export default LoadingButton;
