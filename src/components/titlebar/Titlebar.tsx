import { Component } from "solid-js";
import {
  handleMinimize,
  handleClose,
  handleHide,
  handleMaximize,
} from "../../handlers/titlebar.handlers";

interface TitlebarProps {
  title: string;
  showMaximize?: boolean;
  /** When true, clicking X hides the window instead of closing it (for buddy list / system tray). */
  hideOnClose?: boolean;
}

const Titlebar: Component<TitlebarProps> = (props) => {
  return (
    <div class="xfire-titlebar" data-tauri-drag-region>
      <span class="xfire-titlebar-text" data-tauri-drag-region>
        {props.title}
      </span>
      <div class="xfire-titlebar-spacer" data-tauri-drag-region />
      <button class="xfire-titlebar-btn" onClick={handleMinimize}>
        &#x2014;
      </button>
      {props.showMaximize && (
        <button class="xfire-titlebar-btn" onClick={handleMaximize}>
          &#x25A1;
        </button>
      )}
      <button
        class="xfire-titlebar-btn xfire-titlebar-btn-close"
        onClick={props.hideOnClose ? handleHide : handleClose}
      >
        &#x2715;
      </button>
    </div>
  );
};

export default Titlebar;
