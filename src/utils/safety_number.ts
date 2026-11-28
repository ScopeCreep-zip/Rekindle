/** A safety number's 60 digits as Signal shows them: twelve groups of five
 *  (plan E2.3). Pure: the Host computes the number; this only spaces it. */
export function groupDigits(digits: string): string[] {
  return digits.match(/\d{1,5}/g) ?? [];
}
