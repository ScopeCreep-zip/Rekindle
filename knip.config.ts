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
//   * The real-app WebdriverIO suite (e2e/app) is a separate entry
//     surface; its specs don't show up from src/main.tsx.
//   * `vite-plugin-solid` and `@tailwindcss/vite` are loaded via
//     vite.config.ts (a project entry).
//   * Tauri plugins are dynamically wired by Rust; their JS shims live
//     in node_modules and are imported on demand — false-positives
//     here are listed in `ignoreDependencies`.
const config: KnipConfig = {
  entry: [
    "src/main.tsx",
    // Plan E7.14: WebdriverIO loads the real-app specs by the config's glob.
    "e2e/app/specs/**/*.e2e.ts",
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
    // WebdriverIO loads `runner: "local"` as @wdio/local-runner by name;
    // knip's webdriver-io plugin maps every runner but "local".
    "@wdio/local-runner",
  ],
  // Vite loads the default export when package.json runs `vite build`.
  vite: { config: "vite.config.ts" },
  // WDIO loads the named config from the test:app scripts and the per-spec
  // runs documented in the budget, damage, and idle configs.
  "webdriver-io": { config: ["e2e/app/wdio.conf.ts", "e2e/app/wdio.*.conf.ts"] },
  // Check exports on source entries. Plugins account for exports that the
  // Vite and WebdriverIO runners load from their configuration files.
  includeEntryExports: true,
};

export default config;
