/**
 * A name's initials for an avatar or tile: the first letter of each of its
 * first two words, upper-cased ("e0b4 7e41" → "E7"). Fed the name as the
 * Host serves it, never a label made from it ("Alice (you)" would read
 * "A("), so every avatar and call tile draws the same initials for one
 * person.
 */
export function initials(name: string): string {
  return name
    .split(/\s+/)
    .filter((w) => w.length > 0)
    .map((w) => w[0])
    .join("")
    .toUpperCase()
    .slice(0, 2);
}
