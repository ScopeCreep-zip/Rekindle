import type { SkinAction } from "../ipc/commands/dto_skin_tree";

/** Words for a skin action: its accessible name where the pack draws its label as art. */
export function actionLabel(a: SkinAction): string {
  const words = a.kind.replace(/([A-Z])/g, " $1").toLowerCase();
  const arg = "arg" in a && typeof a.arg === "string" ? ` ${a.arg.replace(/([A-Z])/g, " $1").toLowerCase()}` : "";
  return (words + arg).replace(/^./, (c) => c.toUpperCase());
}
