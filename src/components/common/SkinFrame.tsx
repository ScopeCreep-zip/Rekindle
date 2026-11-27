import { type Component, Index } from "solid-js";

/// The tile slots a skin's window frame draws into (plan SK0 S2). Equal to
/// `rekindle-skin` `shape::MAX_FRAME_TILES`; a test there reads this line.
const FRAME_TILES = 32;
const SLOTS = Array.from({ length: FRAME_TILES });

/// The window's skin outline: one empty slot per frame tile. The Host's
/// served stylesheet (`@layer shape`) places each slot where the skin's
/// layout lays its tile and shows it; a skin without a layout for the
/// window leaves every slot hidden. Decorative, so hidden from assistive
/// technology; the frame band outside the content drags the window like
/// the titlebar. Absolutely positioned, so it is no flex item of the
/// window root (the flex contract is unchanged).
const SkinFrame: Component = () => (
  <div class="skin-frame" aria-hidden="true" data-tauri-drag-region>
    <Index each={SLOTS}>{() => <i data-tauri-drag-region />}</Index>
  </div>
);

export default SkinFrame;
