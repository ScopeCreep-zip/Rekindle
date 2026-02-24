import { Component } from "solid-js";
import { ICON_THREAD } from "../../icons";

interface ThreadStarterProps {
  threadName: string;
  messageCount: number;
  onClick: () => void;
}

const ThreadStarter: Component<ThreadStarterProps> = (props) => {
  return (
    <button class="thread-starter-badge" onClick={props.onClick}>
      <span class="nf-icon">{ICON_THREAD}</span>
      {props.threadName} ({props.messageCount})
    </button>
  );
};

export default ThreadStarter;
