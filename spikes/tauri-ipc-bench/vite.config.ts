import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The bench measures frame pacing, so the dev server must not inject anything
// that competes for the main thread. HMR is off for that reason.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true, hmr: false },
  build: { target: "safari16", sourcemap: true },
});
