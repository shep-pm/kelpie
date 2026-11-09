import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// Port 1520 is kelpie's, not Tauri's template default 1420: see
// .claude/skills/tauri, choice 6. strictPort means a taken port fails the
// dev run instead of quietly moving, which is what slice 3's `build.devUrl`
// has to match.
export default defineConfig({
  clearScreen: false,
  plugins: [react()],
  server: {
    port: 1520,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    // The Tauri window will load this in slice 3 (`build.frontendDist`).
    target: "es2022",
  },
  test: {
    environment: "jsdom",
  },
});
