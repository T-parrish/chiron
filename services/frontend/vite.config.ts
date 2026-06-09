import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": {
        target: process.env.API_TARGET ?? "http://api-gateway:8080",
        rewrite: (path) => path.replace(/^\/api/, ""),
      },
    },
  },
});
