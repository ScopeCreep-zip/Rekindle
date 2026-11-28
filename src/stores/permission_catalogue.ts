import { createResource, type Resource } from "solid-js";
import { commands } from "../ipc/commands";
import type { PermissionEntry, PermissionGroup } from "../ipc/commands/dto_capabilities";

/** A permission of the Host's catalogue with its bit as a BigInt. */
interface CataloguePermission {
  key: string;
  label: string;
  bit: bigint;
}

/** A group of the Host's catalogue, in its order. */
export interface CatalogueGroup {
  name: string;
  permissions: CataloguePermission[];
}

/** A served entry with its decimal bit as a BigInt (the DTO carries it as a
 *  JS-safe decimal string, contract PN1). */
const toPermission = (p: PermissionEntry): CataloguePermission => ({
  key: p.key,
  label: p.label,
  bit: BigInt(p.bit),
});

/** A served group, its permissions in the Host's order. */
const toGroup = (group: PermissionGroup): CatalogueGroup => ({
  name: group.name,
  permissions: group.permissions.map(toPermission),
});

let loading: Promise<CatalogueGroup[]> | null = null;

/** The catalogue, asked of the Host once per window (contract PN1); a
 *  failed read is asked again by the next editor that opens. */
function load(): Promise<CatalogueGroup[]> {
  loading ??= commands.getPermissionCatalogue().then((catalogue) => catalogue.groups.map(toGroup));
  loading.catch(() => {
    loading = null;
  });
  return loading;
}

/** The Host's permission catalogue for an editor: groups, labels and bits. */
export function usePermissionCatalogue(): Resource<CatalogueGroup[]> {
  const [catalogue] = createResource(load);
  return catalogue;
}
