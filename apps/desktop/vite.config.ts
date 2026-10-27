import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import pkg from "./package.json";

// Tauri expects a fixed dev port and must not watch the Rust side. 1430, not Musubiko's 1420,
// so both can run at once.
export default defineConfig({
  plugins: [react()],
  define: { __APP_VERSION__: JSON.stringify(pkg.version) },
  clearScreen: false,
  server: {
    port: 1430,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/bridge/**"] },
  },
  build: {
    target: "safari15",
    outDir: "dist",
  },
});
