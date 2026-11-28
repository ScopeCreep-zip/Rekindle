import { createStore } from "solid-js/store";
import type { DisappearingTimerView } from "../ipc/commands/dto_dm";

/** Each open conversation's disappearing timer (plan E7.1), keyed by the
 *  friend's key or the group's id: what `disappearing_get` answered, then
 *  each `disappearingTimerChanged`. The window decides nothing here. */
export const [disappearingState, setDisappearingState] = createStore<{
  timers: Record<string, DisappearingTimerView>;
}>({ timers: {} });
