import test from "node:test";
import assert from "node:assert/strict";
import {
  activeAttention,
  subjectLabel,
  kindLabel,
  duration,
  WALLBOARD_LIMITS,
  projectPreviewMessages,
} from "../src/pages/wallboard.ts";
import { alertFixture, wallboardFixture } from "./wallboard-fixture.mjs";
import { OperationsClient } from "../src/api/client.ts";

test("wallboard attention retains Phase 127 order and excludes Resolved/Closed", () => {
  const data = wallboardFixture();
  assert.deepEqual(
    activeAttention(data.alerts.value.attention_ordered.items).map(
      (a) => a.lifecycle_status,
    ),
    ["open", "acknowledged", "in_progress"],
  );
});

test("canonical map/proposal identities never collapse into application identity", () => {
  for (const [kind, field] of [
    ["dmap", "dmap_id"],
    ["deployment_proposal", "proposal_id"],
  ]) {
    const a = subjectLabel({
      kind,
      application_id: "same-app",
      [field]: "identity-A",
    });
    const b = subjectLabel({
      kind,
      application_id: "same-app",
      [field]: "identity-B",
    });
    assert.notEqual(a, b);
    assert.match(a, /identity-A/);
    assert.match(b, /identity-B/);
  }
});

for (const [loaded, hasMore, partial] of [
  [6, false, false],
  [15, false, false],
  [10, true, false],
  [15, true, false],
  [6, false, true],
  [10, true, true],
]) {
  test(`project continuation: ${loaded} loaded, upstream ${hasMore}, partial ${partial}`, () => {
    const messages = projectPreviewMessages(loaded, hasMore);
    assert.equal(messages.length, Number(loaded > 10) + Number(hasMore));
    assert.equal(
      messages.some((m) => m.includes("View Projects")),
      loaded > 10,
    );
    assert.equal(
      messages.some((m) => m.includes("pagination is not yet exposed")),
      hasMore,
    );
    // Coverage is independent and does not enter this pagination-only function.
    assert.ok(messages.every((m) => !m.includes("coverage")));
  });
}
test("attention is bounded and refuses active flag/lifecycle disagreement", () => {
  assert.equal(
    activeAttention(
      Array.from({ length: 20 }, (_, i) => alertFixture("open", `a${i}`)),
    ).length,
    8,
  );
  assert.equal(
    activeAttention([
      { ...alertFixture("resolved"), is_operationally_active: true },
    ]).length,
    0,
  );
});
test("subject identity and condition kind are explicit, never human-text authority", () => {
  assert.match(subjectLabel(alertFixture().subject), /mqtt-service/);
  assert.equal(kindLabel("dmap_accepted"), "D-Map accepted");
  assert.equal(kindLabel("new_condition"), "new condition");
});
test("condition confirmations are retained, duration anomaly never becomes zero", () => {
  assert.equal(alertFixture().condition_confirmation_count, 120);
  assert.equal(duration(null), "Time indeterminate");
  assert.equal(duration(-1), "Time indeterminate");
  assert.equal(duration(3600), "1h 0m");
  assert.equal(duration(59), "59s");
});
test("recent occurrence preview is bounded without reordering canonical chronology", () => {
  assert.equal(WALLBOARD_LIMITS.events, 15);
  const events = wallboardFixture().recent_events.value.items;
  assert.equal(
    events.slice(0, WALLBOARD_LIMITS.events).at(-1).event_id,
    "event-014",
  );
});
test("expanded wallboard contract validates available data and rejects contradictory lifecycle", async () => {
  const original = globalThis.fetch;
  const data = wallboardFixture();
  globalThis.fetch = async () => new Response(JSON.stringify(data));
  try {
    assert.equal(
      (await new OperationsClient().overview(new AbortController().signal))
        .alerts.value.counts.open,
      1,
    );
    data.alerts.value.attention_ordered.items[0].is_operationally_active = false;
    await assert.rejects(
      new OperationsClient().overview(new AbortController().signal),
      (e) => e.code === "invalid_response",
    );
  } finally {
    globalThis.fetch = original;
  }
});
test("malformed temporal/event contracts are rejected without exposing raw values", async () => {
  const original = globalThis.fetch;
  const data = wallboardFixture();
  data.recent_events.value.items[0].occurred_at_utc = "secret/path";
  globalThis.fetch = async () => new Response(JSON.stringify(data));
  try {
    await assert.rejects(
      new OperationsClient().overview(new AbortController().signal),
      (e) => e.code === "invalid_response" && !e.message.includes("secret"),
    );
  } finally {
    globalThis.fetch = original;
  }
});
