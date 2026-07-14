import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

// The app is a webview frontend, so @types/node is not a dependency of it -
// but this config file is the one place that runs in node, and it needs the
// one variable Tauri sets for it. Declared, not installed.
declare const process: { env: Record<string, string | undefined> };

// Tauri drives Vite: it runs `npm run dev` and points its webview at devUrl
// (http://localhost:1420), so the port is fixed and strict. clearScreen is off
// so Tauri's own CLI output is not wiped by Vite on startup.
//
// The rest is Tauri's prescribed frontend config (v2.tauri.app/start/frontend/vite):
//
// - build.target: the app ships to a webview, not to a browser matrix. Windows
//   runs Chromium (WebView2), everything else runs WebKit, and the bundle
//   advertises macOS 10.15, whose Safari is 13. Vite's own default baseline is
//   far newer, so without this an untranspiled bit of modern syntax is a blank
//   window on the oldest Mac we claim to support.
// - envPrefix: TAURI_ENV_* is how the frontend can see what it is being built
//   for (platform, arch, debug) through import.meta.env.
// - server.watch.ignored: src-tauri is Rust; a Cargo build touching target/
//   must not trigger a frontend reload.
export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM == "windows" ? "chrome105" : "safari13",
  },
});
