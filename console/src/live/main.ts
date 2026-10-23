import "bootstrap/dist/css/bootstrap.min.css";
import "bootstrap-icons/font/bootstrap-icons.css";
import "../styles/console.css";
import "./live.css";
import { OperationsClient } from "../api/client.ts";
import { ensureAtLeastOneView } from "./live-profile.ts";
import { liveView } from "./live-view.ts";
import { el, badge } from "../components/presentation.ts";

const root = document.querySelector<HTMLDivElement>("#app");
if (!root) throw new Error("DATUM Live root missing");

function createMemoryStorage(): Pick<Storage, "getItem" | "setItem"> {
  const map = new Map<string, string>();
  return {
    getItem: (key) => map.get(key) ?? null,
    setItem: (key, value) => {
      map.set(key, value);
    },
  };
}

// Review P1-6 fix: `window.localStorage` itself (not just `.setItem`) can
// throw -- storage-restricted browser policy, a kiosk/embedded browser
// lockdown, a sandboxed iframe. Live's entire purpose is running
// unattended on a screen nobody is actively watching for a console error,
// so this must degrade to an in-memory, non-persistent fallback rather
// than fail the whole module at evaluation time.
function resolveStorage(): { storage: Pick<Storage, "getItem" | "setItem">; persistent: boolean } {
  try {
    const probeKey = "__datum_live_storage_probe__";
    window.localStorage.setItem(probeKey, "1");
    window.localStorage.removeItem(probeKey);
    return { storage: window.localStorage, persistent: true };
  } catch {
    return { storage: createMemoryStorage(), persistent: false };
  }
}

const { storage, persistent } = resolveStorage();

// A Live's identity lives in `?view=<id>` so two tabs can each pin a
// different profile without one overwriting the other (Phase 142A brief,
// "permita abrir duas Lives diferentes em duas abas"). Server-side
// sharing of this URL across devices is explicitly out of scope (142B).
const profiles = ensureAtLeastOneView(storage);
const url = new URL(window.location.href);
const requestedId = url.searchParams.get("view");
const active = profiles.find((p) => p.id === requestedId) ?? profiles[0]!;
if (persistent && url.searchParams.get("view") !== active.id) {
  url.searchParams.set("view", active.id);
  window.history.replaceState(null, "", url.toString());
}

const client = new OperationsClient();
let current = liveView(storage, client, profiles, active.id, (id) => {
  if (!persistent) return;
  const next = new URL(window.location.href);
  next.searchParams.set("view", id);
  window.history.replaceState(null, "", next.toString());
});

// `replaceChildren` (not `append`) so an HMR re-evaluation of this module
// never leaves a stale previous instance's DOM behind alongside the new one.
if (!persistent) {
  const notice = el("div", "live-storage-notice");
  notice.setAttribute("role", "status");
  notice.append(
    badge("Storage unavailable", "warning"),
    el("span", "small", "Live profile changes in this session will not be saved (browser storage is blocked or unavailable)."),
  );
  root.replaceChildren(notice, current.element);
} else {
  root.replaceChildren(current.element);
}

if (import.meta.hot)
  import.meta.hot.dispose(() => {
    current.dispose();
  });
