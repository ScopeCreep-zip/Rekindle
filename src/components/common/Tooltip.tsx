import type { Component, JSX } from "solid-js";

interface TooltipProps {
  text: string;
  children: JSX.Element;
}

/** Its text shows on hover and while focus is inside it (WCAG 1.4.13), by
 *  the stylesheet. */
const Tooltip: Component<TooltipProps> = (props) => (
  <div class="tooltip-wrapper">
    {props.children}
    <div class="tooltip-bubble" role="tooltip">
      {props.text}
    </div>
  </div>
);

export default Tooltip;
