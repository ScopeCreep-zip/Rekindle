import { createSignal } from "solid-js";
import type { SkinSignalSet } from "../ipc/commands/dto_skin_run";

/**
 * This window's skin signals as the Host computed them (plan SK2-A4 step
 * 3, design §2.5); `null` until the first set arrives. The frontend never
 * computes a capability: it only applies the pack triggers they fire.
 */
const [skinSignals, setSkinSignals] = createSignal<SkinSignalSet | null>(null);

export { setSkinSignals, skinSignals };
