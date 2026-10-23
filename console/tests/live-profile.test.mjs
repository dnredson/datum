import test from "node:test";
import assert from "node:assert/strict";
import {
  loadLiveViews,
  saveLiveViews,
  ensureAtLeastOneView,
  upsertLiveView,
  removeLiveView,
  newLiveView,
  reviveLiveView,
  mergeAndSaveView,
  mergeRemoveAndSaveView,
  LIVE_PROFILE_SCHEMA,
  LIVE_STORE_SCHEMA,
  DEFAULT_REFRESH_INTERVAL_SECONDS,
  MAX_NAME_LENGTH,
} from "../src/live/live-profile.ts";

function memoryStorage(initial) {
  const map = new Map(initial ? Object.entries(initial) : []);
  return {
    getItem: (key) => (map.has(key) ? map.get(key) : null),
    setItem: (key, value) => {
      map.set(key, value);
    },
  };
}

test("empty localStorage yields exactly one default profile, persisted", () => {
  const storage = memoryStorage();
  const views = ensureAtLeastOneView(storage);
  assert.equal(views.length, 1);
  assert.equal(views[0].name, "Default");
  assert.equal(loadLiveViews(storage).length, 1);
});

test("round-trips a saved profile exactly, including nested filters", () => {
  const storage = memoryStorage();
  const view = { ...newLiveView("Lab"), pinned_node_ids: ["node-a", "node-b"], filters: { severities: ["critical"], project_ids: ["p1"] } };
  saveLiveViews(storage, [view]);
  const loaded = loadLiveViews(storage);
  assert.deepEqual(loaded, [view]);
});

test("corrupted JSON in localStorage never throws, yields an empty list", () => {
  const storage = memoryStorage({ "datum.live-views/v1": "{not json" });
  assert.deepEqual(loadLiveViews(storage), []);
});

test("unknown top-level store schema is ignored, never partially trusted", () => {
  const storage = memoryStorage({
    "datum.live-views/v1": JSON.stringify({ schema: "datum.live-views-store/99", views: [newLiveView("X")] }),
  });
  assert.deepEqual(loadLiveViews(storage), []);
});

test("a mix of valid and corrupted profile entries keeps only the valid ones", () => {
  const good = newLiveView("Good");
  const storage = memoryStorage({
    "datum.live-views/v1": JSON.stringify({
      schema: LIVE_STORE_SCHEMA,
      views: [good, { schema: LIVE_PROFILE_SCHEMA }, "not an object", null, 42],
    }),
  });
  const loaded = loadLiveViews(storage);
  assert.equal(loaded.length, 1);
  assert.equal(loaded[0].id, good.id);
});

test("a profile with an out-of-range refresh interval is repaired to the default, not discarded", () => {
  const broken = { ...newLiveView("Broken"), refresh_interval_seconds: -5 };
  const revived = reviveLiveView(broken);
  assert.ok(revived);
  assert.equal(revived.refresh_interval_seconds, DEFAULT_REFRESH_INTERVAL_SECONDS);
});

test("duplicate pinned node ids are de-duplicated while preserving first-seen order", () => {
  const broken = { ...newLiveView("Dup"), pinned_node_ids: ["a", "b", "a", "c", "b"] };
  const revived = reviveLiveView(broken);
  assert.deepEqual(revived.pinned_node_ids, ["a", "b", "c"]);
});

test("empty name or missing id cannot be salvaged -- the entry is dropped, not defaulted", () => {
  assert.equal(reviveLiveView({ schema: LIVE_PROFILE_SCHEMA, id: "x", name: "" }), undefined);
  assert.equal(reviveLiveView({ schema: LIVE_PROFILE_SCHEMA, name: "x" }), undefined);
  assert.equal(reviveLiveView("not an object"), undefined);
  assert.equal(reviveLiveView({ schema: "wrong", id: "x", name: "x" }), undefined);
});

test("upsertLiveView replaces an existing profile by id rather than duplicating it", () => {
  const view = newLiveView("A");
  const renamed = { ...view, name: "A renamed" };
  const result = upsertLiveView([view], renamed);
  assert.equal(result.length, 1);
  assert.equal(result[0].name, "A renamed");
});

test("removeLiveView removes only the targeted profile", () => {
  const a = newLiveView("A");
  const b = newLiveView("B");
  const result = removeLiveView([a, b], a.id);
  assert.deepEqual(result, [b]);
});

test("multiple Lives coexist independently in the same store without collision", () => {
  const storage = memoryStorage();
  const a = newLiveView("Lab");
  const b = newLiveView("TV");
  saveLiveViews(storage, [a, b]);
  const loaded = loadLiveViews(storage);
  assert.equal(loaded.length, 2);
  assert.ok(loaded.some((v) => v.id === a.id && v.name === "Lab"));
  assert.ok(loaded.some((v) => v.id === b.id && v.name === "TV"));
});

test("a full/blocked localStorage (setItem throws) never crashes save", () => {
  const storage = {
    getItem: () => null,
    setItem: () => {
      throw new Error("QuotaExceededError");
    },
  };
  assert.doesNotThrow(() => saveLiveViews(storage, [newLiveView("X")]));
});

test("an excessively long name is truncated to MAX_NAME_LENGTH, not discarded", () => {
  const huge = { ...newLiveView("ok"), name: "x".repeat(MAX_NAME_LENGTH * 10) };
  const revived = reviveLiveView(huge);
  assert.ok(revived);
  assert.equal(revived.name.length, MAX_NAME_LENGTH);
});

// ---- Phase 142A review P1-1 fix: cross-tab/cross-window persistence race ----

test("mergeAndSaveView never clobbers a profile created by another tab between this tab's load and its own save", () => {
  const storage = memoryStorage();
  const initial = newLiveView("Default");
  saveLiveViews(storage, [initial]);

  // Tab 1 and Tab 2 both load the same initial snapshot.
  const tab1Snapshot = loadLiveViews(storage);
  const tab2Snapshot = loadLiveViews(storage);

  // Tab 2 creates a brand-new profile and saves it the SAFE way.
  const createdByTab2 = newLiveView("TV");
  mergeAndSaveView(storage, createdByTab2);

  // Tab 1, unaware of Tab 2's change, edits its own (now stale) copy of
  // Default and saves via the SAFE helper too.
  const editedByTab1 = { ...tab1Snapshot[0], pinned_node_ids: ["node-a"] };
  mergeAndSaveView(storage, editedByTab1);

  // Both changes must survive -- this is exactly what a blind
  // `saveLiveViews(storage, tab1Snapshot)` full-array overwrite would lose.
  const final = loadLiveViews(storage);
  assert.equal(final.length, 2);
  assert.ok(final.some((v) => v.id === createdByTab2.id && v.name === "TV"));
  const finalDefault = final.find((v) => v.id === initial.id);
  assert.deepEqual(finalDefault.pinned_node_ids, ["node-a"]);
  void tab2Snapshot; // documents that Tab 2's own stale snapshot is irrelevant once it saved safely
});

test("mergeRemoveAndSaveView removes only the targeted profile even if the caller's in-memory list is stale", () => {
  const storage = memoryStorage();
  const a = newLiveView("A");
  saveLiveViews(storage, [a]);
  loadLiveViews(storage); // some caller's stale snapshot, intentionally unused below
  const b = newLiveView("B");
  mergeAndSaveView(storage, b); // a second tab adds B after the snapshot above was taken

  const remaining = mergeRemoveAndSaveView(storage, a.id);
  assert.equal(remaining.length, 1);
  assert.equal(remaining[0].id, b.id);
});
