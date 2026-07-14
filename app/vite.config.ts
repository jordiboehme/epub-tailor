import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

// Tauri drives Vite: it runs `npm run dev` and points its webview at devUrl
// (http://localhost:1420), so the port is fixed and strict. clearScreen is off
// so Tauri's own CLI output is not wiped by Vite on startup.
export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
});
