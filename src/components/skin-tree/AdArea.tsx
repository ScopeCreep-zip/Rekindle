import { Component, JSX } from "solid-js";

/**
 * N4 (design §3, §7.1): the pack's ad slot. Xfire's ad was served and is
 * in no pack, so the slot shows a Rekindle banner of its own size (the
 * one content difference, plan SK2).
 */
const AdArea: Component<{ attrs: Record<string, unknown>; children: JSX.Element }> = (props) => (
  <div {...props.attrs} class="skin-ad" role="img" aria-label="Rekindle">
    {props.children}
    <span class="skin-ad-banner" aria-hidden="true">
      Rekindle
    </span>
  </div>
);

export default AdArea;
