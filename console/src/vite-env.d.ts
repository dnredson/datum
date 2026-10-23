/// <reference types="vite/client" />

// Review P2-B: `DATUM_CONSOLE_URL` is injected via `vite.config.ts`'s
// `define` (not `envPrefix`), so it never pulls the rest of the `DATUM_*`
// env family into the client bundle. This augments Vite's own
// `ImportMetaEnv` so the single extra key is known to the type checker.
interface ImportMetaEnv {
  readonly DATUM_CONSOLE_URL?: string;
}
