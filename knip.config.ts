import type { KnipConfig } from "knip";

// Detect unused files, dependencies, and exports.
//
// Run with:  pnpm exec knip
//
// Knip walks the project's import graph from the configured entry points
// and reports anything that isn't reached. CI runs it as a warning gate
// — see .github/workflows/lint.yml.
//
// Notes for Rekindle:
//   * Vite + SolidJS app: entry is src/main.tsx.
//   * Playwright tests are a separate entry surface; their handlers
//     don't show up from src/main.tsx.
//   * `vite-plugin-solid` and `@tailwindcss/vite` are loaded via
//     vite.config.ts (a project entry).
//   * Tauri plugins are dynamically wired by Rust; their JS shims live
//     in node_modules and are imported on demand — false-positives
//     here are listed in `ignoreDependencies`.
const config: KnipConfig = {
  entry: [
    "src/main.tsx",
    "vite.config.ts",
    "playwright.config.ts",
    "e2e/**/*.spec.ts",
    "scripts/copy-sidecar.mjs",
  ],
  project: [
    "src/**/*.{ts,tsx}",
    "scripts/**/*.{ts,mjs}",
    "e2e/**/*.ts",
  ],
  ignore: [
    "src-tauri/**",
    "src-tauri/gen/**",
    "legacy/**",
    "dist/**",
    "node_modules/**",
  ],
  ignoreDependencies: [
    // Tauri JS shims for plugins are imported lazily by name.
    "@tauri-apps/plugin-.*",
    // Tailwind 4 + CSS-first config: the dependency is only referenced
    // from CSS via `@import "tailwindcss"` or vite plugin config.
    "tailwindcss",
    "@tailwindcss/vite",
    // Vite-plugin-solid is referenced from vite.config.ts only.
    "vite-plugin-solid",
  ],
  // SolidJS components register reactivity on import; treat default
  // exports of *.tsx files as in-use even if no static import reaches them.
  includeEntryExports: true,
};

export default config;
