import { type Component, Show } from "solid-js";
import { titleLogo } from "../../hooks/useSkin";
import { BrandGlyph } from "../skin-tree/BrandGlyphs";
import {
  handleMinimize,
  handleClose,
  handleHide,
  handleMaximize,
} from "../../actions/titlebar.actions";

interface TitlebarProps {
  title: string;
  showMaximize?: boolean;
  /** When true, clicking X hides the window instead of closing it (for buddy list / system tray). */
  hideOnClose?: boolean;
}

const Titlebar: Component<TitlebarProps> = (props) => {
  return (
    <div class="xfire-titlebar" data-tauri-drag-region>
      {/* The logo's place (plan SK1-S1): shown and placed by the Host's served
          stylesheet when the skin lays a title strip; it draws the Rekindle
          mark the Host chose (plan BR), its word in the strip's text colour. */}
      <span class="xfire-titlebar-logo" aria-hidden="true" data-tauri-drag-region>
        <Show when={titleLogo()}>{(glyph) => <BrandGlyph glyph={glyph()} />}</Show>
      </span>
      <span class="xfire-titlebar-text" data-tauri-drag-region>
        {props.title}
      </span>
      <div class="xfire-titlebar-spacer" data-tauri-drag-region />
      <button
        type="button"
        class="xfire-titlebar-btn xfire-titlebar-btn-minimize"
        onClick={handleMinimize}
        aria-label="Minimize window"
      >
        <span class="xfire-titlebar-glyph" aria-hidden="true">&#x2014;</span>
      </button>
      {props.showMaximize && (
        <button
          type="button"
          class="xfire-titlebar-btn xfire-titlebar-btn-maximize"
          onClick={handleMaximize}
          aria-label="Maximize window"
        >
          <span class="xfire-titlebar-glyph" aria-hidden="true">&#x25A1;</span>
        </button>
      )}
      <button
        type="button"
        class="xfire-titlebar-btn xfire-titlebar-btn-close"
        onClick={props.hideOnClose ? handleHide : handleClose}
        aria-label={props.hideOnClose ? "Hide window" : "Close window"}
      >
        <span class="xfire-titlebar-glyph" aria-hidden="true">&#x2715;</span>
      </button>
    </div>
  );
};

export default Titlebar;
