import { Component, createMemo, createSignal, For, Show } from "solid-js";
import type { SafetyNumberView } from "../../ipc/commands/dto";
import { handleSetVerified } from "../../actions/verification.actions";
import { addToast } from "../../stores/toast.store";
import { groupDigits } from "../../utils/safety_number";
import VerifiedBadge from "./VerifiedBadge";

/** Modules of light margin a scanner needs around a QR code. */
const QUIET_ZONE = 4;

/** The dark modules of `rows` as one SVG path, offset by the quiet zone. */
function qrPath(rows: string[]): string {
  let d = "";
  rows.forEach((row, y) => {
    for (let x = 0; x < row.length; x++) {
      if (row[x] === "1") d += `M${x + QUIET_ZONE} ${y + QUIET_ZONE}h1v1h-1z`;
    }
  });
  return d;
}

/**
 * Our safety number with a peer (plan E2.3): the 60 digits in twelve groups
 * of five, their QR code in Signal's scannable format, the verified state,
 * and the mark-verified flow. Marking and clearing go through the Host's
 * review (TrustChanging): its checkbox states the number, and committing it
 * is the user saying they compared it.
 *
 * A security surface (plan E7.7.0): the number and the code sit on a fixed
 * card drawn with the reserved `--sec-*` tokens, and the QR is drawn as a
 * path from the Host's module rows (no markup is injected), so no skin can
 * hide, recolour or fake what the user compares.
 */
const SafetyNumberPanel: Component<{
  peerKey: string;
  view: SafetyNumberView | null;
  onChanged: () => void;
}> = (props) => {
  const [busy, setBusy] = createSignal(false);
  const groups = createMemo(() => groupDigits(props.view?.digits ?? ""));
  const size = createMemo(() => (props.view?.qrRows.length ?? 0) + 2 * QUIET_ZONE);

  async function setVerified(verified: boolean): Promise<void> {
    setBusy(true);
    const error = await handleSetVerified(props.peerKey, verified);
    setBusy(false);
    if (error !== null) addToast(error, "error");
    props.onChanged();
  }

  return (
    <Show when={props.view} fallback={<div class="sec-safety-number">Reading the safety number…</div>}>
      {(view) => (
        <section class="sec-safety-number" aria-label="Safety number">
          <div class="sec-safety-number-state">
            <VerifiedBadge view={view()} />
            <Show when={!view().verified && !view().changed}>Not verified</Show>
          </div>
          <Show when={view().changed}>
            <p class="sec-safety-number-hint">
              Their key changed since you verified it. Compare the new number before you mark it
              verified again.
            </p>
          </Show>
          <div class="sec-safety-number-digits" aria-label={`Safety number ${groups().join(" ")}`}>
            <For each={groups()}>{(group) => <span aria-hidden="true">{group}</span>}</For>
          </div>
          <svg
            class="sec-safety-number-qr"
            viewBox={`0 0 ${size()} ${size()}`}
            shape-rendering="crispEdges"
            role="img"
            aria-label="Safety number QR code"
          >
            <rect class="sec-safety-number-qr-ground" width={size()} height={size()} />
            <path class="sec-safety-number-qr-modules" d={qrPath(view().qrRows)} />
          </svg>
          <p class="sec-safety-number-hint">
            Compare these numbers with them in person or on a call you trust, or scan their code.
          </p>
          <button
            type="button"
            class="sec-btn-secondary"
            disabled={busy()}
            onClick={() => void setVerified(!view().verified)}
          >
            {view().verified ? "Clear verification" : "Mark as verified"}
          </button>
        </section>
      )}
    </Show>
  );
};

export default SafetyNumberPanel;
