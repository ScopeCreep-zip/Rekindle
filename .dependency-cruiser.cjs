/* eslint-disable */
// dependency-cruiser configuration — frontend tier hierarchy.
//
// Spec: https://github.com/sverweij/dependency-cruiser
//
// Run with:  pnpm exec depcruise --config .dependency-cruiser.cjs src
//
// Visualise:
//   pnpm exec depcruise --config .dependency-cruiser.cjs --output-type dot src \
//     | dot -Tsvg > deps.svg
//
// CI gate: .github/workflows/lint.yml `frontend-arch` job, which now
// runs it for real — `dependency-cruiser` is a devDependency and the
// step's "is the binary present" guard is gone. Before that the job
// existed but its only step always skipped, so these rules documented
// the boundary rather than enforcing it.
//
// Tiers (lower may be imported by higher; never the reverse):
//
//   src/ipc/         (leaf)         the Tauri boundary; typed invoke wrappers
//   src/utils/       (leaf)         time / format / colour helpers
//   src/icons.ts     (leaf)         icon enum
//   src/styles/      (leaf)         CSS only — never imported as TS
//
//   src/stores/                     reactive signals + IPC calls; no business logic
//   src/actions/                    user-action dispatchers (`handle*`) — what a
//                                   click calls; reads stores, invokes IPC
//   src/handlers/                   event *subscription registration* (`subscribe*`)
//                                   — wired at app/window start, never by a click
//
//   src/components/                 presentation only
//   src/windows/                    one component per Tauri window; composes components
//
// `src/actions/` was split out of `src/handlers/`, which held both roles
// under one name and produced 64 of this config's violations. A button
// calling `handleAddReaction` is ordinary in any frontend; a component
// importing event-subscription wiring is not — and while both lived in
// one directory (behind barrels that re-exported both), the two were
// indistinguishable at the import site. Splitting them made the rule
// satisfiable instead of something to baseline.
//
// Forbidden patterns (rules below):
//
//   * components / handlers / stores → @tauri-apps/api or @tauri-apps/plugin-*
//   * components → handlers          (a click never registers a subscription)
//   * actions → components, windows  (actions are below presentation)
//   * stores → components, windows
//   * ipc → stores, actions, handlers, components, windows  (ipc is leaf)
//
// Windows *may* import handlers: several subscriptions are window-scoped
// (BuddyListWindow's voice presence, DmWindow's inbox), so the older
// "windows → handlers is forbidden" note described an intent the code
// never had.

/** @type {import('dependency-cruiser').IConfiguration} */
module.exports = {
  forbidden: [
    // ─── No direct Tauri imports outside src/ipc/ ────────────────
    {
      name: "no-tauri-api-outside-ipc",
      severity: "error",
      comment:
        "Direct imports of @tauri-apps/api defeat the typed wrapper layer. " +
        "Use src/ipc/commands.ts or src/ipc/channels.ts.",
      // `pathNot` rather than a `(?!ipc/)` lookahead: dependency-cruiser
      // 18 refuses lookahead in a path regex as ReDoS-unsafe and bails
      // out of the whole run, which is one way this gate could have kept
      // reporting success while checking nothing.
      from: {
        path: "^src/",
        pathNot: "^src/ipc/",
      },
      to: {
        path: "^@tauri-apps/api($|/)",
      },
    },
    {
      name: "no-tauri-plugin-outside-ipc",
      severity: "error",
      comment:
        "Direct imports of Tauri plugins defeat the typed wrapper layer. " +
        "Wrap the plugin in a thin module under src/ipc/ first.",
      from: {
        path: "^src/",
        pathNot: "^src/ipc/",
      },
      to: {
        path: "^@tauri-apps/plugin-",
        // `@tauri-apps/plugin-opener` is the documented path for safe
        // external-URL navigation — see frontend-rendering.md gate 4.
        pathNot: "^@tauri-apps/plugin-opener$",
      },
    },

    // ─── No ipc → upper tier ─────────────────────────────────────
    {
      name: "actions-no-presentation",
      severity: "error",
      comment:
        "Action dispatchers sit below presentation: they read stores and " +
        "invoke IPC. A component or window importing an action is the " +
        "supported direction; the reverse is not.",
      from: {
        path: "^src/actions/",
      },
      to: {
        path: "^src/(components|windows)/",
      },
    },
    {
      name: "ipc-is-leaf",
      severity: "error",
      comment:
        "src/ipc/ is the bottom tier — typed wrappers around invoke/listen. " +
        "Importing from stores/handlers/components creates a cycle and " +
        "smuggles business logic into the IPC layer.",
      from: {
        path: "^src/ipc/",
      },
      to: {
        path: "^src/(stores|actions|handlers|components|windows)/",
      },
    },

    // ─── No stores → components / windows ────────────────────────
    {
      name: "stores-no-presentation",
      severity: "error",
      comment:
        "Stores are reactive state + IPC. They must not import UI " +
        "components — that creates a cycle and pulls the renderer " +
        "into the data layer.",
      from: {
        path: "^src/stores/",
      },
      to: {
        path: "^src/(components|windows)/",
      },
    },

    // ─── No handlers → components / windows ──────────────────────
    {
      name: "handlers-no-presentation",
      severity: "error",
      comment:
        "Handlers register channel listeners and dispatch to stores. " +
        "They must not import components or windows.",
      from: {
        path: "^src/handlers/",
      },
      to: {
        path: "^src/(components|windows)/",
      },
    },

    // ─── No components → handlers ────────────────────────────────
    {
      name: "components-no-handlers",
      severity: "error",
      comment:
        "Handlers are an app-startup concern, not a per-component one. " +
        "Components subscribe to store state, not to channels directly.",
      from: {
        path: "^src/components/",
      },
      to: {
        path: "^src/handlers/",
      },
    },

    // ─── Components may not import other windows ─────────────────
    {
      name: "components-no-windows",
      severity: "error",
      comment:
        "Windows compose components, never the reverse. Components must " +
        "be reusable across windows.",
      from: {
        path: "^src/components/",
      },
      to: {
        path: "^src/windows/",
      },
    },

    // ─── No localStorage / sessionStorage ────────────────────────
    {
      name: "no-direct-storage",
      severity: "error",
      comment:
        "Browser localStorage / sessionStorage bypasses Stronghold + " +
        "@tauri-apps/plugin-store integrity. Use the Tauri store plugin " +
        "via a wrapper in src/ipc/.",
      from: {
        path: "^src/",
      },
      to: {
        path: "^(localStorage|sessionStorage)$",
      },
    },

    // ─── No raw fetch ────────────────────────────────────────────
    {
      name: "no-raw-fetch",
      severity: "warn",
      comment:
        "Raw fetch() bypasses Tauri's command system. If you need to " +
        "talk to a remote endpoint, do it from the Rust backend.",
      from: {
        path: "^src/(components|stores|handlers)/",
      },
      to: {
        path: "^node-fetch$|^cross-fetch$|^isomorphic-fetch$|^undici$",
      },
    },

    // ─── No circular dependencies ────────────────────────────────
    {
      name: "no-circular",
      severity: "error",
      comment: "Circular dependencies are a refactor smell.",
      from: {},
      to: {
        circular: true,
      },
    },

    // ─── No orphan modules (warn) ────────────────────────────────
    {
      name: "no-orphans",
      severity: "warn",
      comment:
        "Orphan modules are unreachable from the entry point — likely " +
        "dead code. knip catches most of these too; this rule is the " +
        "tier-aware backstop.",
      from: {
        orphan: true,
        // Every pattern here is single-quantifier by construction.
        // dependency-cruiser rejects nested quantifiers as ReDoS-unsafe
        // and aborts the entire run on the first one it sees, so a
        // `(?:\\.\\w+)*\\.` here silently disabled all 200-odd lines of
        // tier rules rather than just this one.
        pathNot: [
          "(^|/)\\.[^/.]+\\.(?:js|cjs|mjs|ts|tsx|json)$",
          "\\.d\\.ts$",
          "(^|/)tsconfig\\.[^/.]+\\.json$",
          "(^|/)(?:babel|webpack|vite|playwright|knip|biome)[^/]*\\.(?:js|cjs|mjs|ts|json)$",
        ],
      },
      to: {},
    },

    // ─── No deprecated core modules ──────────────────────────────
    {
      name: "no-deprecated-core",
      severity: "warn",
      comment: "Don't import deprecated Node core modules.",
      from: {},
      to: {
        dependencyTypes: ["core"],
        path: "^(punycode|domain|constants|sys|_linklist|_stream_wrap)$",
      },
    },
  ],

  options: {
    doNotFollow: {
      path: "node_modules",
    },
    exclude: {
      path: [
        "node_modules",
        "dist",
        "src-tauri",
        "target",
        "legacy",
        "e2e/security",
        "playwright-report",
        "test-results",
      ],
    },
    moduleSystems: ["es6", "cjs"],
    tsPreCompilationDeps: true,
    tsConfig: {
      fileName: "tsconfig.json",
    },
    enhancedResolveOptions: {
      exportsFields: ["exports"],
      conditionNames: ["import", "require", "node", "default", "types"],
      mainFields: ["module", "main", "types", "typings"],
    },
    reporterOptions: {
      dot: {
        collapsePattern:
          "^(?:packages|src|lib|app|bin|test(s?)|spec(s?))/[^/]+|node_modules/[^/]+",
      },
      archi: {
        collapsePattern:
          "^(packages|src|lib|app|bin|test(s?)|spec(s?))/[^/]+|node_modules/[^/]+",
      },
      text: {
        highlightFocused: true,
      },
    },
  },
};
