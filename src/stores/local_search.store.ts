import { createSignal } from "solid-js";
import type { LocalSearchScope } from "../ipc/commands/dto_local_search";

/** The scope the open local search looks in (the Host's `OpenSearch`, plan E7.2 G11), or none: closed. */
const [localSearchScope, setLocalSearchScope] = createSignal<LocalSearchScope | null>(null);

export { localSearchScope, setLocalSearchScope };
