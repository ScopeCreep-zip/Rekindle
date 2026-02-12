import { Component, JSX } from "solid-js";

interface ScrollAreaProps {
  children: JSX.Element;
  class?: string;
}

const ScrollArea: Component<ScrollAreaProps> = (props) => {
  return (
    <div class={`scroll-area ${props.class || ""}`}>
      {props.children}
    </div>
  );
};

export default ScrollArea;
