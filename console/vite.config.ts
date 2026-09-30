import { defineConfig, loadEnv } from "vite";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "DATUM_");
  return {
    server: {
      host: "127.0.0.1",
      port: 5173,
      strictPort: true,
      proxy: {
        "/api": { target: env.DATUM_DEV_API_TARGET || "http://127.0.0.1:8080" },
      },
    },
    // Preview is a local integration tool, not a production server.
    preview: {
      proxy: {
        "/api": { target: env.DATUM_DEV_API_TARGET || "http://127.0.0.1:8080" },
      },
    },
  };
});
