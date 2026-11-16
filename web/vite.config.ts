import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In development the UI runs on :5173 and proxies the API, the Robine ID
// sign-in routes and the logs intake to the Rust server.
const backend = process.env.ROBINE_DOG_BACKEND ?? "http://127.0.0.1:8080";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      "/api": backend,
      "/auth": backend,
      "/health": backend,
    },
  },
});
