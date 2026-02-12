import { Component, JSX, createSignal, Show } from "solid-js";

interface TooltipProps {
  text: string;
  children: JSX.Element;
}

const Tooltip: Component<TooltipProps> = (props) => {
  const [visible, setVisible] = createSignal(false);

  return (
    <div
      class="tooltip-wrapper"
      onMouseEnter={() => setVisible(true)}
      onMouseLeave={() => setVisible(false)}
    >
      {props.children}
      <Show when={visible()}>
        <div class="tooltip-bubble">{props.text}</div>
      </Show>
    </div>
  );
};

export default Tooltip;
