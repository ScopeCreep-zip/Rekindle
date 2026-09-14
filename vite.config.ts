import path from "node:path";
import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [solid(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "src"),
    },
  },

  // Expose Tauri env vars to frontend
  envPrefix: ["VITE_", "TAURI_ENV_*"],

  // Pre-bundle all known deps to eliminate on-demand discovery
  optimizeDeps: {
    include: [
      "@tauri-apps/api",
      "@tauri-apps/api/core",
      "@tauri-apps/api/event",
      "@tauri-apps/api/window",
      "@tauri-apps/api/mocks",
      "@tauri-apps/plugin-autostart",
      "@tauri-apps/plugin-deep-link",
      "@tauri-apps/plugin-notification",
      "@tauri-apps/plugin-process",
      "@tauri-apps/plugin-store",
      "@tauri-apps/plugin-stronghold",
      "solid-js",
      "solid-js/web",
      "solid-js/store",
    ],
  },

  clearScreen: false,
  server: {
    // 1430 (not the Tauri scaffold default 1420) so Rekindle's dev server
    // coexists with other Tauri/Vite projects on this machine instead of
    // colliding on 1420 — a collision silently loaded the wrong app into
    // the Rekindle shell. Keep in sync with tauri.conf.json devUrl,
    // package.json `dev`, the mac/linux dev scripts, and playwright.
    port: 1430,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1431,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
    // Pre-transform entry point and hot paths on server start
    warmup: {
      clientFiles: [
        "./src/main.tsx",
        "./src/windows/LoginWindow.tsx",
        "./src/stores/*.ts",
        "./src/ipc/*.ts",
        "./src/handlers/auth.handlers.ts",
        "./src/styles/global.css",
      ],
    },
  },

  build: {
    target:
      process.env.TAURI_ENV_PLATFORM === "windows"
        ? "chrome105"
        : "safari13",
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    chunkSizeWarningLimit: 1024,
    reportCompressedSize: false,
  },
});
