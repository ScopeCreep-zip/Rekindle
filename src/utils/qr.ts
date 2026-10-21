/** Modules of light margin a scanner needs around a QR code. */
export const QUIET_ZONE = 4;

/** The side of the square SVG viewBox for `rows` with its quiet zone. */
export function qrSize(rows: string[]): number {
  return rows.length + 2 * QUIET_ZONE;
}

/** The dark modules of the Host's QR `rows` (`1` dark) as one SVG path,
 *  offset by the quiet zone, so no markup is ever injected. */
export function qrPath(rows: string[]): string {
  let d = "";
  rows.forEach((row, y) => {
    for (let x = 0; x < row.length; x++) {
      if (row[x] === "1") d += `M${x + QUIET_ZONE} ${y + QUIET_ZONE}h1v1h-1z`;
    }
  });
  return d;
}
