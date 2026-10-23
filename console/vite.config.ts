import { defineConfig, loadEnv } from "vite";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "DATUM_");
  return {
    // Review P2-B fix (replaces the earlier P1-5 fix's `envPrefix:
    // ["VITE_", "DATUM_"]`, which exposed the ENTIRE `DATUM_*` env family
    // to the client bundle): inject only the one key the Live bundle
    // actually needs, as a static replacement of the literal
    // `import.meta.env.DATUM_CONSOLE_URL` expression. `envPrefix` is left
    // at Vite's own default (`VITE_` only) -- a future `DATUM_*` var is
    // never automatically exposed to client code just by existing.
    define: {
      "import.meta.env.DATUM_CONSOLE_URL": JSON.stringify(env.DATUM_CONSOLE_URL ?? ""),
    },
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
    // Phase 142A -- two independent entry points from one Vite project:
    // the DATUM Console (`index.html`) and DATUM Live (`live.html`). Both
    // share every TypeScript contract/type/API client under `src/`; this
    // is the only build-level change multi-page requires. In production
    // each is served from its own subdomain (console.datumlab.net /
    // live.datumlab.net) by pointing Nginx at the matching built HTML file
    // -- a deployment detail, not an application contract.
    build: {
      rollupOptions: {
        input: {
          console: "index.html",
          live: "live.html",
        },
      },
    },
  };
});
