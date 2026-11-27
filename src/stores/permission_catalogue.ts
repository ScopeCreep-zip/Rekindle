import { createResource, type Resource } from "solid-js";
import { commands } from "../ipc/commands";

/** A permission of the Host's catalogue with its bit as a BigInt. */
export interface CataloguePermission {
  key: string;
  label: string;
  bit: bigint;
}

/** A group of the Host's catalogue, in its order. */
export interface CatalogueGroup {
  name: string;
  permissions: CataloguePermission[];
}

let loading: Promise<CatalogueGroup[]> | null = null;

/** The catalogue, asked of the Host once per window (contract PN1); a
 *  failed read is asked again by the next editor that opens. */
function load(): Promise<CatalogueGroup[]> {
  loading ??= commands.getPermissionCatalogue().then((catalogue) =>
    catalogue.groups.map((group) => ({
      name: group.name,
      permissions: group.permissions.map((p) => ({ key: p.key, label: p.label, bit: BigInt(p.bit) })),
    })),
  );
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
