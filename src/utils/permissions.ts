// Bit maths for the role and overwrite editors. Which permissions exist,
// their groups and labels are the Host's (get_permission_catalogue, contract
// PN1: stores/permission_catalogue.ts), and what we may do is the Host's too
// (CAPR's denied maps: utils/denied.ts). The shell declares no permission.

/** Whether `perms` holds every bit of `bits` (an editor's checkbox). */
export function hasBits(perms: bigint, bits: bigint): boolean {
  return (perms & bits) === bits;
}

/** `current` with `bit` flipped (an editor's toggle). */
export function togglePermBit(current: bigint, bit: bigint): bigint {
  return current ^ bit;
}
