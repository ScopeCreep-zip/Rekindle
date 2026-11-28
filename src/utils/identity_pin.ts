/**
 * The Host's refusal when a friend's prekey record is not signed by the
 * identity we hold for them (`prekeys/pick.rs`, `IdentityPinMismatch`,
 * plan E2.4 B3), shown on their row and in place of the chat input.
 */
export function identityPinMismatchText(name: string): string {
  return `${name}'s keys are not signed by the identity you know. Nothing was sent; verify their safety number.`;
}
