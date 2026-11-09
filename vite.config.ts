import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// Port 1520 is kelpie's, not Tauri's template default 1420: see
// .claude/skills/tauri, choice 6. strictPort means a taken port fails the
// dev run instead of quietly moving, which is what slice 3's `build.devUrl`
// has to match.
export default defineConfig({
  clearScreen: false,
  plugins: [react(), tailwindcss()],
  // `@/` imports resolve through tsconfig.json `paths`, the same alias
  // components.json gives shadcn.
  resolve: {
    tsconfigPaths: true,
  },
  server: {
    port: 1520,
    strictPort: true,
    // `src-tauri` is a workspace member, so `tauri dev` writes its build output
    // to the root `target/`. Vite would watch every file in it.
    watch: {
      ignored: ["**/target/**"],
    },
  },
  // Vite matches each prefix with `key.startsWith(prefix)`, so a trailing `*`
  // matches no variable name at all. `TAURI_ENV_` is what lets slice 3 read
  // `import.meta.env.TAURI_ENV_PLATFORM`.
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // The Tauri window will load this in slice 3 (`build.frontendDist`).
    target: "es2022",
  },
  test: {
    environment: "jsdom",
  },
});
