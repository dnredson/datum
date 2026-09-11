#!/usr/bin/env python3
"""Self-tests for `scripts/check_datum_contract_fixtures.py`.

Covers:
  * the TD-1 / ADR-0012 lifecycle in `evaluate_td1`, across all three states
    (Deferred/Open, Accepted Option A, Accepted Option B) and the
    option-dependent rule for immediate-revocation claims;
  * negative-fixture code matching: a negative fixture proves its intended
    failure only when its `x_expect_reject.code` is among the produced finding
    codes -- an unrelated validation error is not enough;
  * D-Forward chain-hop resolution.

Python 3 standard library only.

Run:
    python scripts/check_datum_contract_fixtures_selftest.py
    python -m unittest scripts.check_datum_contract_fixtures_selftest
"""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location(
    "datum_stage107_checker", _HERE / "check_datum_contract_fixtures.py")
chk = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(chk)


# --- ADR-0012 text builders ------------------------------------------------- #

DEFERRED_BODY = (
    "Immediate revocation must not be claimed until a TD-1 option is specified. "
    "No document, ADR, fixture, evidence record or test may describe v1 (or v2) "
    "authorization as providing immediate revocation of already-decided mutations.\n"
    "Must be decided before Stage 107A implementation begins.\n"
)


def adr(status: str, *, resolution: str | None = None, obligations: bool = False,
        optionb_evidence: bool = False, body: str = DEFERRED_BODY) -> str:
    lines = ["# ADR-0012 -- TOCTOU", f"- Status: {status}"]
    if resolution:
        lines.append(f"- TD-1 resolution: {resolution}")
    if optionb_evidence:
        lines.append("- TD-1 Option B evidence: satisfied")
    lines.append("")
    lines.append(body)
    if obligations:
        lines.append("\n## Selected semantics and test obligations\n"
                     "The selected semantics carries these test obligations: "
                     "lease bound honoured; batches never mix generations; ...\n")
    return "\n".join(lines)


CLAIM = "This design provides immediate revocation of not-yet-executed mutations."
CLAIM2 = "An artifact change immediately prevents all not-yet-executed mutations."
SAFE_A = ("It provides an immutable authorized snapshot for a bounded lease; "
          "changes take effect no later than lease expiry (bounded staleness).")


def fails(adr_text: str, scanned: dict[str, str] | None = None):
    return chk.evaluate_td1(adr_text, scanned or {})


def where_msgs(result):
    return [f"{w}: {m}" for w, m in result]


# --- tests ---------------------------------------------------------------- #

class Td1Deferred(unittest.TestCase):
    def test_deferred_clean_passes(self):
        self.assertEqual(fails(adr("**DEFERRED / OPEN**")), [])

    def test_deferred_with_resolution_fails(self):
        r = fails(adr("Deferred / Open", resolution="Option A"))
        self.assertTrue(any("records a '- TD-1 resolution:'" in m for _, m in r), r)

    def test_deferred_missing_noclaim_constraint_fails(self):
        r = fails(adr("Deferred / Open",
                      body="Must be decided before Stage 107A implementation begins.\n"))
        self.assertTrue(any("must not be" in m for _, m in r), r)

    def test_deferred_missing_before_107a_marker_fails(self):
        r = fails(adr("Deferred / Open",
                      body="Immediate revocation must not be claimed. No document may claim it.\n"))
        self.assertTrue(any("before Stage 107A" in m for _, m in r), r)

    def test_deferred_claim_in_a_doc_fails(self):
        r = fails(adr("Deferred / Open"), {"some-doc.md": CLAIM})
        self.assertEqual(where_msgs(r),
                         ["some-doc.md: asserts immediate revocation "
                          "('provides immediate revocation') which the current "
                          "TD-1 state does not permit"])

    def test_deferred_no_claim_in_docs_passes(self):
        self.assertEqual(fails(adr("Deferred / Open"), {"d.md": SAFE_A}), [])


class Td1AcceptedOptionA(unittest.TestCase):
    def test_option_a_clean_passes(self):
        self.assertEqual(
            fails(adr("Accepted", resolution="Option A", obligations=True),
                  {"model.md": SAFE_A}), [])

    def test_option_a_immediate_revocation_claim_fails(self):  # requirement 2
        r = fails(adr("Accepted", resolution="Option A", obligations=True),
                  {"model.md": CLAIM})
        self.assertTrue(any("does not permit" in m for _, m in r), r)

    def test_option_a_prevents_wording_also_fails(self):
        r = fails(adr("Accepted", resolution="Option A", obligations=True),
                  {"model.md": CLAIM2})
        self.assertTrue(any("does not permit" in m for _, m in r), r)

    def test_option_a_missing_obligations_fails(self):
        r = fails(adr("Accepted", resolution="Option A", obligations=False))
        self.assertTrue(any("no test obligations" in m for _, m in r), r)


class Td1AcceptedOptionB(unittest.TestCase):
    def test_option_b_no_evidence_no_claim_passes(self):
        self.assertEqual(
            fails(adr("Accepted", resolution="Option B", obligations=True),
                  {"d.md": SAFE_A}), [])

    def test_option_b_no_evidence_with_claim_fails(self):  # requirement 3 (negative)
        r = fails(adr("Accepted", resolution="Option B", obligations=True),
                  {"d.md": CLAIM})
        self.assertTrue(any("does not permit" in m for _, m in r), r)

    def test_option_b_with_evidence_and_claim_passes(self):  # requirement 3 (positive)
        self.assertEqual(
            fails(adr("Accepted", resolution="Option B", obligations=True,
                      optionb_evidence=True),
                  {"d.md": "Once implemented, Option B provides immediate revocation "
                           "of not-yet-executed mutations."}), [])

    def test_option_b_missing_resolution_line_fails(self):
        r = fails(adr("Accepted", resolution=None, obligations=True))
        self.assertTrue(any("no '- TD-1 resolution:" in m for _, m in r), r)


class Td1StatusParsing(unittest.TestCase):
    def test_unknown_status_fails(self):
        r = fails(adr("Proposed"))
        self.assertTrue(any("not recognised" in m for _, m in r), r)

    def test_adr_status_helper(self):
        self.assertEqual(chk.adr_status("- Status: **DEFERRED / OPEN**"), "deferred / open")
        self.assertEqual(chk.adr_status("- Status: Accepted"), "accepted")


class Td1RepoState(unittest.TestCase):
    """Stage 107A resolves TD-1: the real ADR-0012 is Accepted / Option A,
    passes the lifecycle checker cleanly, and still rejects an
    immediate-revocation claim injected into any other document."""

    def _adr_text(self) -> str:
        toctou = chk.ADR_DIR / "ADR-0012-toctou-authorization-validity.md"
        self.assertTrue(toctou.exists())
        return toctou.read_text(encoding="utf-8")

    def test_real_adr_0012_and_repo_docs_are_clean(self):
        result = chk.evaluate_td1(self._adr_text(), chk._td1_scan_targets())
        self.assertEqual(where_msgs(result), [])

    def test_real_adr_0012_is_accepted_option_a(self):
        text = self._adr_text()
        self.assertEqual(chk.adr_status(text), "accepted")
        self.assertRegex(text, r"(?im)^\s*[-*]\s*TD-1 resolution:\s*Option A\b")
        self.assertIn("test obligations", text.lower())

    def test_real_option_a_still_rejects_immediate_revocation_claim(self):
        # K.27: Option A accepted, but an assertive immediate-revocation claim in
        # any scanned doc/fixture is still a checker failure.
        result = chk.evaluate_td1(self._adr_text(),
                                  {"injected.md": CLAIM, "injected2.md": CLAIM2})
        self.assertTrue(any("does not permit" in m for _, m in result), result)
        self.assertEqual(len(result), 2, result)


# --- negative-fixture code matching (correction 9) ------------------------- #

class NegativeFixtureCodeMatch(unittest.TestCase):
    def test_expected_code_present_passes(self):
        ok, reason = chk.negative_fixture_verdict(
            "duplicate_dserv_id", {"duplicate_dserv_id", "dserv_missing_dcode"})
        self.assertTrue(ok, reason)

    def test_only_unrelated_error_fails(self):
        ok, reason = chk.negative_fixture_verdict(
            "duplicate_dserv_id", {"dserv_missing_dcode"})
        self.assertFalse(ok)
        self.assertIn("not the expected 'duplicate_dserv_id'", reason)

    def test_no_findings_fails(self):
        ok, reason = chk.negative_fixture_verdict("duplicate_dserv_id", set())
        self.assertFalse(ok)
        self.assertIn("unexpectedly passed", reason)

    def test_missing_expected_code_field_fails(self):
        ok, reason = chk.negative_fixture_verdict(None, {"duplicate_dserv_id"})
        self.assertFalse(ok)
        self.assertIn("missing x_expect_reject.code", reason)

    def test_unknown_expected_code_fails(self):
        ok, reason = chk.negative_fixture_verdict("not_a_real_code", {"not_a_real_code"})
        self.assertFalse(ok)
        self.assertIn("not a known rejection code", reason)

    def test_known_reject_codes_are_all_emittable(self):
        self.assertEqual(chk.KNOWN_REJECT_CODES - chk.EMITTABLE_CODES, set())

    def test_every_real_negative_fixture_asserts_its_code(self):
        import json
        idx = _load_positive_index()
        for path in sorted(chk.NEG_DIR.glob("*.json")):
            obj = json.loads(path.read_text(encoding="utf-8"))
            want = (obj.get("x_expect_reject") or {}).get("code")
            produced = chk.finding_codes(chk.dispatch_findings(obj, idx))
            ok, reason = chk.negative_fixture_verdict(want, produced)
            self.assertTrue(ok, f"{path.name}: {reason}")

    def test_every_positive_fixture_is_finding_free(self):
        import json
        idx = _load_positive_index()
        for path in sorted(chk.FIXTURE_DIR.glob("*.json")):
            obj = json.loads(path.read_text(encoding="utf-8"))
            findings = chk.dispatch_findings(obj, idx)
            self.assertEqual(findings, [], f"{path.name}: {findings}")


# --- D-Forward chain-hop resolution (correction 8) ----------------------- #

class DForwardChainHops(unittest.TestCase):
    def test_repo_dforward_chain_hops_resolve(self):
        import json
        obj = json.loads((chk.FIXTURE_DIR / "dforward.monitor-f49c8.json")
                         .read_text(encoding="utf-8"))
        codes = chk.finding_codes(chk.v_dforward(obj))
        self.assertNotIn("chain_hop_unresolved", codes)

    def test_dangling_chain_hop_is_reported(self):
        obj = {
            "schema": "datum.dforward/1", "dforward_id": "dforward:x", "revision": 1,
            "diots": [{"diot_id": "a", "diot_kind": "k", "artifact": {}, "provides": [], "requires": []}],
            "external_endpoints": [],
            "chains": [{"chain_id": "c", "hops": [{"from": "a", "to": "ghost", "interface": "i"}]}],
        }
        codes = chk.finding_codes(chk.v_dforward(obj))
        self.assertIn("chain_hop_unresolved", codes)

    def test_external_endpoint_resolves_a_hop(self):
        obj = {
            "schema": "datum.dforward/1", "dforward_id": "dforward:x", "revision": 1,
            "diots": [{"diot_id": "a", "diot_kind": "k", "artifact": {}, "provides": [], "requires": []}],
            "external_endpoints": [{"endpoint_id": "field-radio", "note": "real gateway"}],
            "chains": [{"chain_id": "c", "hops": [{"from": "field-radio", "to": "a", "interface": "i"}]}],
        }
        codes = chk.finding_codes(chk.v_dforward(obj))
        self.assertNotIn("chain_hop_unresolved", codes)


def _load_positive_index() -> dict:
    import json
    idx = {"dgraph": {}, "dcontinuum": {}, "dforward": {}, "dserv_artifact": {}}
    for path in sorted(chk.FIXTURE_DIR.glob("*.json")):
        obj = json.loads(path.read_text(encoding="utf-8"))
        s = obj.get("schema", "")
        if s == "datum.dgraph/1":
            idx["dgraph"][(obj.get("dgraph_id"), obj.get("revision"))] = obj
        elif s == "datum.dcontinuum/1":
            idx["dcontinuum"][(obj.get("continuum_id"), obj.get("revision"))] = obj
        elif s == "datum.dforward/1":
            idx["dforward"][(obj.get("dforward_id"), obj.get("revision"))] = obj
        elif s == "datum.dserv-artifact/1":
            idx["dserv_artifact"][obj.get("artifact_id")] = obj
    return idx


class Stage107AMapperDigest(unittest.TestCase):
    def fixture(self):
        import json
        return json.loads((chk.FIXTURE_DIR / chk.MAPPER_FIXTURE).read_text(encoding="utf-8"))

    def test_actual_mapper_fixture_has_matching_canonical_digest_carriers(self):
        self.assertTrue(chk.mapper_fixture_digest_valid(self.fixture()))

    def test_bare_digest_or_changed_canonical_bytes_are_rejected(self):
        fixture = self.fixture()
        fixture["expected_projection_digest_sha256"] = fixture["expected_projection_digest_sha256"][7:]
        self.assertFalse(chk.mapper_fixture_digest_valid(fixture))
        fixture = self.fixture()
        fixture["expected_canonical_json"] += " "
        self.assertFalse(chk.mapper_fixture_digest_valid(fixture))


if __name__ == "__main__":
    unittest.main(verbosity=2)
