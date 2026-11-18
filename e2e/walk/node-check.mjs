// Plan E7.13: the walk's Node floor, checked before walk.ts is loaded. An
// older Node cannot load walk.ts at all (Node 18 stops at
// ERR_UNKNOWN_FILE_EXTENSION), so the check is plain JavaScript run first.
// The runner needs .ts type stripping without a flag (Node 23.6.0, backported
// to 22.18.0) and import.meta.dirname (Node 20.11.0, launch.ts:15).

const [major, minor] = process.versions.node.split(".").map(Number);
const ok = major > 23 || (major === 23 && minor >= 6) || (major === 22 && minor >= 18);
if (!ok) {
  console.error(
    `[walk] Node ${process.versions.node} cannot run the walk: it needs Node 22.18+ or 23.6+ ` +
      "(TypeScript type stripping on by default, and import.meta.dirname). Install a newer Node and rerun pnpm walk.",
  );
  process.exit(1);
}
