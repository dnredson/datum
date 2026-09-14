#!/usr/bin/env python3
"""Self-tests for `scripts/check_ddeploy_contracts_v1.py`.

Covers:
  * negative-fixture code matching (same discipline as the Stage 107/110
    checkers' selftests);
  * every real negative fixture under `ddeploy-v1/negative/` declares a
    known, emittable `x_expect_reject.code`;
  * the ONE deliberate digest-order divergence this ADR records:
    `digest_of_proposal` is ORDER-SENSITIVE over `placements` (mirrors
    `compute_proposal_digest`, which never sorts), while `digest_of_dmap`
    is ORDER-INDEPENDENT (mirrors `recompute_dmap_digest`, Stage 110, which
    sorts by `service_id` before hashing) -- proven side by side so a future
    edit cannot silently make one match the other's behavior by accident;
  * proposal-context mismatch detection (project/application) and the
    stale-revision narration check;
  * acceptance cross-check catches a tampered `proposal_digest`;
  * schema-ID uniqueness actually detects a regression (a third fixture
    family reclaiming a production schema ID), not merely passes by
    coincidence today.

Python 3 standard library only.

Run:
    python scripts/check_ddeploy_contracts_v1_selftest.py
"""

from __future__ import annotations

import copy
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location(
    "datum_ddeploy_v1_checker", _HERE / "check_ddeploy_contracts_v1.py")
chk = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(chk)


def codes(findings):
    return {f["code"] for f in findings}


def sample_proposal():
    return {
        "schema": chk.DDEPLOY_PROPOSAL_SCHEMA,
        "proposal_id": "ddeploy-proposal:x",
        "project_id": "proj-x",
        "application_id": "app-x",
        "source": "operator",
        "created_at_utc": "2026-01-01T00:00:00Z",
        "expected_active_dmap_revision": None,
        "dgraph_ref": {"id": "dgraph:x", "revision": 1, "digest": "sha256:" + "a" * 64},
        "dcontinuum_ref": {"id": "continuum:x", "revision": 1, "digest": "sha256:" + "b" * 64},
        "placements": [
            {"service_id": "svc-a", "node_id": "fog-01"},
            {"service_id": "svc-b", "node_id": "cloud-01"},
        ],
    }


def sample_dmap():
    return {
        "schema": chk.CANONICAL_DMAP_SCHEMA,
        "project_id": "proj-x",
        "application_id": "app-x",
        "dmap_id": "dmap:proj-x:app-x",
        "revision": 1,
        "dgraph_ref": {"id": "dgraph:x", "revision": 1, "digest": "sha256:" + "a" * 64},
        "dcontinuum_ref": {"id": "continuum:x", "revision": 1, "digest": "sha256:" + "b" * 64},
        "placements": [
            {"service_id": "svc-a", "node_id": "fog-01"},
            {"service_id": "svc-b", "node_id": "cloud-01"},
        ],
    }


class NegativeFixtureCodeMatch(unittest.TestCase):
    def test_expected_code_present_passes(self):
        ok, _ = chk.negative_fixture_verdict("stale_active_dmap_revision", {"stale_active_dmap_revision"})
        self.assertTrue(ok)

    def test_only_unrelated_error_fails(self):
        ok, reason = chk.negative_fixture_verdict("stale_active_dmap_revision", {"missing_field"})
        self.assertFalse(ok)
        self.assertIn("missing_field", reason)

    def test_no_findings_fails(self):
        ok, reason = chk.negative_fixture_verdict("node_not_eligible", set())
        self.assertFalse(ok)
        self.assertIn("unexpectedly passed", reason)

    def test_missing_expected_code_field_fails(self):
        ok, reason = chk.negative_fixture_verdict(None, {"node_not_eligible"})
        self.assertFalse(ok)
        self.assertIn("missing", reason)

    def test_unknown_expected_code_fails(self):
        ok, reason = chk.negative_fixture_verdict("not_a_real_code", {"not_a_real_code"})
        self.assertFalse(ok)
        self.assertIn("not a known emittable code", reason)

    def test_every_real_negative_fixture_asserts_its_code(self):
        neg_dir = chk.NEG_DIR
        self.assertTrue(neg_dir.exists())
        found_any = False
        for path in sorted(neg_dir.glob("*.json")):
            found_any = True
            obj = json.loads(path.read_text(encoding="utf-8"))
            want = (obj.get("x_expect_reject") or {}).get("code")
            self.assertIsNotNone(want, f"{path.name} is missing x_expect_reject.code")
            self.assertIn(want, chk.EMITTABLE_CODES, f"{path.name} declares an unemittable code '{want}'")
        self.assertTrue(found_any, "expected at least one negative fixture to exist")


class ProposalDigestIsOrderSensitiveUnlikeDMapDigest(unittest.TestCase):
    """The ONE deliberate divergence this ADR records (ADR-0018): a
    proposal's digest is NOT presorted (mirrors `compute_proposal_digest`,
    which hashes `serde_json::to_value(proposal)` verbatim), while a D-Map's
    digest IS presorted by `service_id` (mirrors `recompute_dmap_digest`,
    Stage 110/ADR-0017). A future edit that accidentally made these behave
    identically would silently break either the Stage 111 CAS-adjacent
    identity guarantees or the Stage 110 order-independence guarantee this
    checker is supposed to keep proving apart."""

    def test_proposal_digest_changes_when_placement_order_changes(self):
        p1 = sample_proposal()
        p2 = copy.deepcopy(p1)
        p2["placements"] = list(reversed(p2["placements"]))
        self.assertNotEqual(chk.digest_of_proposal(p1), chk.digest_of_proposal(p2), "compute_proposal_digest hashes placements in the GIVEN order -- reordering them must change the digest")

    def test_dmap_digest_is_order_independent_for_placements(self):
        d1 = sample_dmap()
        d2 = copy.deepcopy(d1)
        d2["placements"] = list(reversed(d2["placements"]))
        self.assertEqual(chk.digest_of_dmap(d1), chk.digest_of_dmap(d2), "recompute_dmap_digest sorts placements by service_id before hashing -- reordering them must NOT change the digest")

    def test_proposal_digest_changes_on_semantic_content_change(self):
        p1 = sample_proposal()
        p2 = copy.deepcopy(p1)
        p2["placements"][0]["node_id"] = "cloud-01"
        self.assertNotEqual(chk.digest_of_proposal(p1), chk.digest_of_proposal(p2))

    def test_proposal_digest_whitelist_drops_bookkeeping_fields(self):
        p1 = sample_proposal()
        p2 = copy.deepcopy(p1)
        p2["x_note"] = "this is fixture bookkeeping, never hashed"
        p2["expected_digest_sha256"] = "sha256:" + "f" * 64
        self.assertEqual(chk.digest_of_proposal(p1), chk.digest_of_proposal(p2), "x_-prefixed / expected_digest_sha256 bookkeeping fields must never affect the recomputed digest")


class ProposalContextMismatchDetection(unittest.TestCase):
    def test_project_mismatch_detected(self):
        p = sample_proposal()
        p["x_request_context_project_id"] = "some-other-project"
        self.assertIn("proposal_project_mismatch", codes(chk.v_proposal_context(p)))

    def test_application_mismatch_detected(self):
        p = sample_proposal()
        p["x_request_context_application_id"] = "some-other-app"
        self.assertIn("proposal_application_mismatch", codes(chk.v_proposal_context(p)))

    def test_matching_context_produces_no_findings(self):
        p = sample_proposal()
        p["x_request_context_project_id"] = p["project_id"]
        p["x_request_context_application_id"] = p["application_id"]
        self.assertEqual(chk.v_proposal_context(p), [])

    def test_stale_revision_detected(self):
        p = sample_proposal()
        p["expected_active_dmap_revision"] = 1
        p["x_current_active_dmap_revision_at_accept_time"] = 2
        self.assertIn("stale_active_dmap_revision", codes(chk.v_proposal_context(p)))

    def test_matching_revision_is_not_stale(self):
        p = sample_proposal()
        p["expected_active_dmap_revision"] = 1
        p["x_current_active_dmap_revision_at_accept_time"] = 1
        self.assertEqual(chk.v_proposal_context(p), [])


class AcceptanceCrossCheckCatchesTampering(unittest.TestCase):
    def test_digest_mismatch_detected_against_real_referenced_proposal(self):
        proposal = sample_proposal()
        real_digest = chk.digest_of_proposal(proposal)
        acceptance = {
            "proposal_id": proposal["proposal_id"],
            "proposal_digest": "sha256:" + "0" * 64,
        }
        self.assertNotEqual(acceptance["proposal_digest"], real_digest)
        findings = chk.v_acceptance_cross_check(acceptance, {proposal["proposal_id"]: proposal})
        self.assertIn("proposal_digest_mismatch", codes(findings))

    def test_matching_digest_produces_no_findings(self):
        proposal = sample_proposal()
        acceptance = {
            "proposal_id": proposal["proposal_id"],
            "proposal_digest": chk.digest_of_proposal(proposal),
        }
        self.assertEqual(chk.v_acceptance_cross_check(acceptance, {proposal["proposal_id"]: proposal}), [])

    def test_unknown_referenced_proposal_is_skipped_not_falsely_flagged(self):
        acceptance = {"proposal_id": "does-not-exist", "proposal_digest": "sha256:" + "0" * 64}
        self.assertEqual(chk.v_acceptance_cross_check(acceptance, {}), [])


class SchemaIdUniquenessDetectsRegression(unittest.TestCase):
    """Mirrors `check_canonical_datum_contracts_v1_selftest.py`'s
    `SchemaIdUniqueness` proof: this check must actually DETECT a schema-ID
    collision, not merely pass because none currently exists."""

    def test_a_fixture_outside_ddeploy_v1_reclaiming_a_production_schema_id_is_caught(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            other_family = root / "some-other-family"
            other_family.mkdir()
            (other_family / "rogue.json").write_text(json.dumps({"schema": chk.DDEPLOY_PROPOSAL_SCHEMA}), encoding="utf-8")

            original_root = chk.DATUM_CONTRACTS_ROOT
            original_fixture_dir = chk.FIXTURE_DIR
            try:
                chk.DATUM_CONTRACTS_ROOT = root
                chk.FIXTURE_DIR = root / "ddeploy-v1"  # deliberately does not exist -- the rogue file is truly outside it
                rep = chk.Report()
                chk.check_schema_id_uniqueness(rep)
                self.assertTrue(any("rogue.json" in f for f in rep.failures), f"expected a violation naming rogue.json, got: {rep.failures}")
            finally:
                chk.DATUM_CONTRACTS_ROOT = original_root
                chk.FIXTURE_DIR = original_fixture_dir

    def test_real_repository_fixtures_pass_cleanly(self):
        rep = chk.Report()
        chk.check_schema_id_uniqueness(rep)
        self.assertEqual(rep.failures, [], f"real repository ddeploy-v1 fixtures must not collide with any other fixture family: {rep.failures}")


class ActiveDeploymentAuthoritySeparation(unittest.TestCase):
    """ADR-0018's authorization-separation invariant, checked at the fixture
    level: `operational_dgraph_id` must never equal the canonical D-Graph's
    own id, nor the legacy 'dgraph:<project_id>' convention."""

    def sample_active_deployment(self):
        dmap = sample_dmap()
        return {
            "project_id": "proj-x",
            "application_id": "app-x",
            "active_dmap": dmap,
            "dmap_digest": chk.digest_of_dmap(dmap),
            "proposal_id": "ddeploy-proposal:x",
            "acceptance_id": "ddeploy-acceptance:x",
            "accepted_at_utc": "2026-01-01T00:00:00Z",
            "operational_dgraph_id": "ddeploy-operational:proj-x",
            "operational_revision": 1,
        }

    def test_valid_active_deployment_passes(self):
        self.assertEqual(chk.v_active_deployment_shape(self.sample_active_deployment()), [])

    def test_operational_id_equal_to_canonical_dgraph_id_is_rejected(self):
        active = self.sample_active_deployment()
        active["operational_dgraph_id"] = active["active_dmap"]["dgraph_ref"]["id"]
        self.assertIn("candidate_dmap_invalid", codes(chk.v_active_deployment_shape(active)))

    def test_operational_id_equal_to_legacy_convention_is_rejected(self):
        active = self.sample_active_deployment()
        active["operational_dgraph_id"] = f"dgraph:{active['project_id']}"
        self.assertIn("candidate_dmap_invalid", codes(chk.v_active_deployment_shape(active)))


class MainScriptRunsCleanOnRealFixtures(unittest.TestCase):
    def test_main_returns_zero(self):
        self.assertEqual(chk.main([]), 0)


if __name__ == "__main__":
    unittest.main()
