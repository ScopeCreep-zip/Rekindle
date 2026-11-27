import { type Component, type JSX, Show, createSignal, onCleanup } from "solid-js";
import { Dynamic } from "solid-js/web";
import type { NodeGlyph, NodeLabel } from "../../ipc/commands/dto_skin_tree";
import { BrandGlyph } from "./BrandGlyphs";

/**
 * The words the Host serves on a node it placed (plan SK2-N4d): over the
 * node's art, centred in `cover` (filled with the skin's ground so the copied
 * art's own word is hidden) or in the whole box. Presses pass through to
 * the node; its accessible name stays the action's.
 */
const NodeLabelView: Component<{ label: NodeLabel }> = (props) => {
  const box = (): JSX.CSSProperties | undefined => {
    const c = props.label.cover;
    return c
      ? { "--label-x": `${c[0]}px`, "--label-y": `${c[1]}px`, "--label-w": `${c[2]}px`, "--label-h": `${c[3]}px` }
      : undefined;
  };
  // A word one size too wide for its box (SK5: "Rekindle" in Symbiosis's
  // and Alloy's 44-45 px titles at 11 px) is drawn a size smaller; a box too
  // narrow even then still ellipsizes. Measured at 11 px, again on resize.
  const [tight, setTight] = createSignal(false);
  const fitText = (el: HTMLSpanElement): void => {
    const measure = (): void => {
      setTight(false);
      queueMicrotask(() => setTight(el.scrollWidth > el.clientWidth + 0.5));
    };
    const ro = new ResizeObserver(measure);
    ro.observe(el.parentElement ?? el);
    onCleanup(() => ro.disconnect());
  };
  return (
    <Show
      when={props.label.glyph}
      fallback={
        <span
          class="skin-node-label"
          data-cover={props.label.cover ? "" : undefined}
          data-tight={tight() ? "" : undefined}
          style={box()}
          aria-hidden="true"
        >
          <span class="skin-node-label-text" ref={fitText}>
            {props.label.text}
          </span>
        </span>
      }
    >
      {(glyph) => (
        <span
          class="skin-node-label skin-node-glyph"
          data-cover={props.label.cover ? "" : undefined}
          style={box()}
          title={props.label.text}
        >
          <NodeGlyphView glyph={glyph()} />
          <span class="sr-only">{props.label.text}</span>
        </span>
      )}
    </Show>
  );
};

/** The community mark (plan SK3): three people, in the current colour. */
const CommunitiesGlyph: Component = () => (
  <svg class="skin-node-glyph-svg" viewBox="0 0 24 24" aria-hidden="true">
    <circle cx="12" cy="7" r="3.2" fill="currentColor" />
    <path d="M6.5 19c0-3.2 2.5-5.6 5.5-5.6s5.5 2.4 5.5 5.6z" fill="currentColor" />
    <circle cx="5" cy="9.5" r="2.3" fill="currentColor" opacity="0.8" />
    <path d="M1 18.5c0-2.4 1.8-4.2 4-4.2 1 0 1.9.3 2.6.9-1.2 1-2 2.3-2.3 3.8z" fill="currentColor" opacity="0.8" />
    <circle cx="19" cy="9.5" r="2.3" fill="currentColor" opacity="0.8" />
    <path d="M23 18.5c0-2.4-1.8-4.2-4-4.2-1 0-1.9.3-2.6.9 1.2 1 2 2.3 2.3 3.8z" fill="currentColor" opacity="0.8" />
  </svg>
);

/**
 * Each built-in glyph's drawing (plan SK3, BR): the community icon in the
 * current colour, the Rekindle marks in theirs, centred in the node's box
 * with their aspect kept. A `Record` over every `NodeGlyph`, so a variant the
 * Host adds without a drawing fails tsc rather than rendering nothing.
 */
const NODE_GLYPHS: Record<NodeGlyph, Component> = {
  communities: CommunitiesGlyph,
  rekindle: () => <BrandGlyph glyph="rekindle" />,
  rekindleMarkSmall: () => <BrandGlyph glyph="rekindleMarkSmall" />,
  rekindleWordmark: () => <BrandGlyph glyph="rekindleWordmark" />,
};

/** A built-in glyph, sized by its box. */
const NodeGlyphView: Component<{ glyph: NodeGlyph }> = (props) => <Dynamic component={NODE_GLYPHS[props.glyph]} />;

export default NodeLabelView;
