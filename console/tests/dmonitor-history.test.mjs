// Phase 140A -- structural regressions for the historical read client. No
// DOM harness is available in this environment (same documented reason as
// every other view in this codebase), so the client itself (read-only,
// single-method) is what is unit/structurally tested here.
import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

async function sourceWithoutComments(path) {
  const source = await readFile(new URL(path, import.meta.url), "utf8");
  return source
    .split("\n")
    .filter((line) => !line.trim().startsWith("//"))
    .join("\n");
}

test("DMonitorHistoryClient exposes exactly one method -- query -- read-only, GET-only, never a mutating verb", async () => {
  const code = await sourceWithoutComments("../src/api/dmonitor-history.ts");
  const methodNames = [...code.matchAll(/\n\s+async (\w+)\(/gu)].map((m) => m[1]);
  assert.deepEqual(methodNames, ["query"], "DMonitorHistoryClient must expose exactly one method: query");
  assert.ok(code.includes('method: "GET"'), "the request must be a GET");
  assert.ok(!code.includes('method: "POST"') && !code.includes('method: "PUT"') && !code.includes('method: "DELETE"'));
});

test("dmonitor-history.ts and dmonitor-history-model.ts never reference host-execution or mutation primitives", async () => {
  for (const path of ["../src/api/dmonitor-history.ts", "../src/pages/dmonitor-history-model.ts"]) {
    const code = await sourceWithoutComments(path);
    for (const forbidden of ["child_process", "ssh2", "Deno.Command", "docker run", "exec(", "spawn(", "createBatch(", "authorize(", "acceptProposal", "createProposal"]) {
      assert.ok(!code.includes(forbidden), `${path} must never reference '${forbidden}'`);
    }
  }
});

// Part: Historical Explorer is a VIEW, never a second monitoring model --
// it must never compute/recompute convergence or read placement authority
// client-side.
test("dmonitor-history-model.ts never references convergence or placement authority", async () => {
  const code = await sourceWithoutComments("../src/pages/dmonitor-history-model.ts");
  for (const forbidden of ["DMonitorConvergence", "AuthorityCorrelation", "active_placement_authority", "PlacementClient"]) {
    assert.ok(!code.includes(forbidden), `dmonitor-history-model.ts must never reference '${forbidden}'`);
  }
});

// The client never filters by PID/container-id/monitor_instance_id --
// logical identity only, mirroring the backend's own deliberate
// restriction (Part A).
test("DMonitorHistoryQuery never exposes a PID/container-id/monitor-instance filter dimension", async () => {
  const code = await sourceWithoutComments("../src/types/dmonitor-history.ts");
  const start = code.indexOf("export interface DMonitorHistoryQuery");
  const body = code.slice(start, code.indexOf("}", start));
  for (const forbidden of ["pid", "containerId", "monitorInstanceId", "processId"]) {
    assert.ok(!body.toLowerCase().includes(forbidden.toLowerCase()), `DMonitorHistoryQuery must never expose '${forbidden}' as a filter`);
  }
});
