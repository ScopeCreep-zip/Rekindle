import { Component, Match, Switch } from "solid-js";
import type { SafetyNumberView } from "../../ipc/commands/dto";
import { ICON_CHECK, ICON_KEY } from "../../icons";

/**
 * A peer's key trust at a glance (plan E2.3): "Verified" once the user
 * marked their safety number compared, "Key changed" while a key they had
 * verified changed and no action acknowledged it yet; nothing for a key
 * never verified. A security surface (plan E7.7.0): its colours are the
 * reserved `--sec-*` ones, so no skin can fake or hide it, and its words
 * are the cue, never the colour alone.
 */
const VerifiedBadge: Component<{ view: SafetyNumberView | null }> = (props) => (
  <Switch>
    <Match when={props.view?.changed}>
      <span class="sec-key-changed-badge" role="status">
        <span class="nf-icon" aria-hidden="true">{ICON_KEY}</span> Key changed
      </span>
    </Match>
    <Match when={props.view?.verified}>
      <span class="sec-verified-badge" role="status">
        <span class="nf-icon" aria-hidden="true">{ICON_CHECK}</span> Verified
      </span>
    </Match>
  </Switch>
);

export default VerifiedBadge;
