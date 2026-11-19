import { Component, JSX } from "solid-js";

interface ScrollAreaProps {
  children: JSX.Element;
  class?: string;
  ref?: (el: HTMLDivElement) => void;
}

const ScrollArea: Component<ScrollAreaProps> = (props) => {
  return (
    <div ref={props.ref} class={`scroll-area ${props.class || ""}`}>
      {props.children}
    </div>
  );
};

export default ScrollArea;
