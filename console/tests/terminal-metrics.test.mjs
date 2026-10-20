import test from "node:test";
import assert from "node:assert/strict";

import { terminalBar } from "../src/components/terminal-metrics.ts";

test("terminalBar renders deterministic bounded ASCII gauges", () => {
  assert.equal(terminalBar(0), "[░░░░░░░░░░░░░░░░░░]");
  assert.equal(terminalBar(0.5), "[█████████░░░░░░░░░]");
  assert.equal(terminalBar(1), "[██████████████████]");
});

test("terminalBar saturates visual fill without changing caller values", () => {
  assert.equal(terminalBar(2.25), "[██████████████████]");
  assert.equal(terminalBar(-1), "[░░░░░░░░░░░░░░░░░░]");
  assert.equal(terminalBar(null), "[··················]");
});
