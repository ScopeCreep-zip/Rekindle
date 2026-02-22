export function colorIntToHex(n: number): string {
  if (!n) return "#000000";
  return `#${(n & 0xFFFFFF).toString(16).padStart(6, "0")}`;
}

export function hexToColorInt(hex: string): number {
  return parseInt(hex.replace("#", ""), 16);
}
