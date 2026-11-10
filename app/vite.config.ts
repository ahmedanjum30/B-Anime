import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [solid()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // WebView2 (Chromium) on Windows; modern WebKit elsewhere.
    target: "es2022",
    cssMinify: true,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
