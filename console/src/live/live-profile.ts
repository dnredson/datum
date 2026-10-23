// Phase 142A -- DATUM Live local profile storage. Strictly browser-local
// (`localStorage`); server-side persistence/sharing of Live Views is
// explicitly out of scope (reserved for a future Phase 142B). A profile
// never carries authority or canonical state -- it is only a saved
// selection of which existing D-Nodes an operator wants to watch, plus
// presentation preferences.
export const LIVE_PROFILE_SCHEMA = "datum.live-view/1" as const;
export const LIVE_STORE_SCHEMA = "datum.live-views-store/1" as const;
const STORAGE_KEY = "datum.live-views/v1";

export type AlertSeverity = "info" | "warning" | "high" | "critical";

export interface LiveViewFilters {
  severities?: AlertSeverity[];
  project_ids?: string[];
}

export interface LiveViewV1 {
  schema: typeof LIVE_PROFILE_SCHEMA;
  id: string;
  name: string;
  pinned_node_ids: string[];
  filters?: LiveViewFilters;
  refresh_interval_seconds: number;
  kiosk_mode: boolean;
  hide_secondary_controls: boolean;
  created_at_utc: string;
  updated_at_utc: string;
}

export interface LiveProfileStoreV1 {
  schema: typeof LIVE_STORE_SCHEMA;
  views: LiveViewV1[];
}

export const DEFAULT_REFRESH_INTERVAL_SECONDS = 15;
export const MIN_REFRESH_INTERVAL_SECONDS = 5;
export const MAX_REFRESH_INTERVAL_SECONDS = 300;
export const MAX_NAME_LENGTH = 80;

function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function text(value: unknown): value is string {
  return typeof value === "string";
}
function stringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every(text);
}
const SEVERITIES: AlertSeverity[] = ["info", "warning", "high", "critical"];

function validFilters(value: unknown): LiveViewFilters | undefined {
  if (value === undefined) return undefined;
  if (!object(value)) return undefined;
  const filters: LiveViewFilters = {};
  if (
    Array.isArray(value.severities) &&
    value.severities.every((s) => SEVERITIES.includes(s as AlertSeverity))
  )
    filters.severities = value.severities as AlertSeverity[];
  if (stringArray(value.project_ids)) filters.project_ids = value.project_ids;
  return filters;
}

function dedupe(ids: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const id of ids) {
    if (seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  return out;
}

// Individually validates and repairs ONE stored profile. Returns undefined
// only when the entry cannot be salvaged at all (not an object, wrong
// schema, missing identity) -- every other field is defensively
// normalized rather than rejecting the whole profile, since a single bad
// field (e.g. a corrupted refresh interval) should not discard an
// operator's saved node selection.
export function reviveLiveView(value: unknown): LiveViewV1 | undefined {
  if (!object(value)) return undefined;
  if (value.schema !== LIVE_PROFILE_SCHEMA) return undefined;
  if (!text(value.id) || value.id.length === 0) return undefined;
  if (!text(value.name) || value.name.length === 0) return undefined;
  const name = value.name.length > MAX_NAME_LENGTH ? value.name.slice(0, MAX_NAME_LENGTH) : value.name;
  const pinned = stringArray(value.pinned_node_ids) ? dedupe(value.pinned_node_ids) : [];
  const refresh =
    typeof value.refresh_interval_seconds === "number" &&
    Number.isFinite(value.refresh_interval_seconds) &&
    value.refresh_interval_seconds >= MIN_REFRESH_INTERVAL_SECONDS &&
    value.refresh_interval_seconds <= MAX_REFRESH_INTERVAL_SECONDS
      ? value.refresh_interval_seconds
      : DEFAULT_REFRESH_INTERVAL_SECONDS;
  const createdAt = text(value.created_at_utc) ? value.created_at_utc : new Date().toISOString();
  const updatedAt = text(value.updated_at_utc) ? value.updated_at_utc : createdAt;
  return {
    schema: LIVE_PROFILE_SCHEMA,
    id: value.id,
    name,
    pinned_node_ids: pinned,
    filters: validFilters(value.filters),
    refresh_interval_seconds: refresh,
    kiosk_mode: value.kiosk_mode === true,
    hide_secondary_controls: value.hide_secondary_controls === true,
    created_at_utc: createdAt,
    updated_at_utc: updatedAt,
  };
}

export function newLiveView(name: string): LiveViewV1 {
  const now = new Date().toISOString();
  return {
    schema: LIVE_PROFILE_SCHEMA,
    id: typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : `live-${now}-${Math.random().toString(36).slice(2)}`,
    name,
    pinned_node_ids: [],
    refresh_interval_seconds: DEFAULT_REFRESH_INTERVAL_SECONDS,
    kiosk_mode: false,
    hide_secondary_controls: false,
    created_at_utc: now,
    updated_at_utc: now,
  };
}

// Tolerates: missing key, corrupted JSON, wrong top-level schema, and a
// mix of valid/invalid individual entries -- never throws, never discards
// an entire store over one bad profile.
export function loadLiveViews(storage: Pick<Storage, "getItem">): LiveViewV1[] {
  let raw: string | null;
  try {
    raw = storage.getItem(STORAGE_KEY);
  } catch {
    return [];
  }
  if (!raw) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!object(parsed) || parsed.schema !== LIVE_STORE_SCHEMA || !Array.isArray(parsed.views)) return [];
  const views: LiveViewV1[] = [];
  for (const candidate of parsed.views) {
    const revived = reviveLiveView(candidate);
    if (revived) views.push(revived);
  }
  return views;
}

export function saveLiveViews(storage: Pick<Storage, "setItem">, views: LiveViewV1[]): void {
  const store: LiveProfileStoreV1 = { schema: LIVE_STORE_SCHEMA, views };
  try {
    storage.setItem(STORAGE_KEY, JSON.stringify(store));
  } catch {
    // A full/blocked localStorage must never crash the page; the in-memory
    // profile stays usable for this session, it simply will not persist.
  }
}

export function upsertLiveView(views: LiveViewV1[], updated: LiveViewV1): LiveViewV1[] {
  const next = views.filter((v) => v.id !== updated.id);
  next.push({ ...updated, updated_at_utc: new Date().toISOString() });
  return next;
}

export function removeLiveView(views: LiveViewV1[], id: string): LiveViewV1[] {
  return views.filter((v) => v.id !== id);
}

// Ensures there is always at least one profile to show -- an empty saved
// state is not an error state, it is a fresh install.
export function ensureAtLeastOneView(
  storage: Pick<Storage, "getItem" | "setItem">,
): LiveViewV1[] {
  const existing = loadLiveViews(storage);
  if (existing.length > 0) return existing;
  const created = [newLiveView("Default")];
  saveLiveViews(storage, created);
  return created;
}

// Phase 142A review P1-1 fix: a caller holding an in-memory `profiles`
// snapshot from an earlier load must NEVER write that stale snapshot back
// wholesale -- a second tab/window may have created or edited a different
// profile in the meantime, and a blind overwrite would silently erase it.
// These two functions always re-read the CURRENT store immediately before
// writing, so only the one profile actually being changed is touched.
export function mergeAndSaveView(
  storage: Pick<Storage, "getItem" | "setItem">,
  updated: LiveViewV1,
): LiveViewV1[] {
  const current = loadLiveViews(storage);
  const merged = upsertLiveView(current, updated);
  saveLiveViews(storage, merged);
  return merged;
}
export function mergeRemoveAndSaveView(
  storage: Pick<Storage, "getItem" | "setItem">,
  id: string,
): LiveViewV1[] {
  const current = loadLiveViews(storage);
  const merged = removeLiveView(current, id);
  saveLiveViews(storage, merged);
  return merged;
}
