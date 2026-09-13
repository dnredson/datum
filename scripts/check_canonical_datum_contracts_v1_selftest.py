#!/usr/bin/env python3
"""Self-tests for `scripts/check_canonical_datum_contracts_v1.py`.

Covers:
  * negative-fixture code matching (same discipline as the Stage 107
    checker's selftest): a negative fixture proves its intended failure only
    when its `x_expect_reject.code` is among the produced finding codes;
  * D-4 (ADR-0001): a D-Call cycle, including a self-call, is never rejected
    by `v_canonical_dgraph`;
  * duplicate detection is not merely a consecutive-elements check (hidden by
    reordering);
  * the digest functions are order-independent for services/calls/nodes/
    links/capabilities/placements, and change when semantic content changes;
  * the digest whitelist actually drops `x_`-prefixed/bookkeeping fields
    (proving the Python/Rust digest parity check in the main script is not
    accidentally comparing apples to oranges).

Python 3 standard library only.

Run:
    python scripts/check_canonical_datum_contracts_v1_selftest.py
"""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location(
    "datum_canonical_v1_checker", _HERE / "check_canonical_datum_contracts_v1.py")
chk = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(chk)


def codes(findings):
    return {f["code"] for f in findings}


def sample_dgraph():
    return {
        "schema": chk.CANONICAL_DGRAPH_SCHEMA,
        "application_id": "app:x",
        "dgraph_id": "dgraph:x",
        "revision": 1,
        "services": [
            {"service_id": "a", "execution_requirement": {"dnode_abi": chk.DNODE_ABI_V0}, "resource_requirement": {"cpu_cores": 1, "memory_bytes": 134217728}},
            {"service_id": "b", "execution_requirement": {"dnode_abi": chk.DNODE_ABI_V0}, "resource_requirement": {"cpu_cores": 1, "memory_bytes": 134217728}},
        ],
        "calls": [
            {"call_id": "a-to-b", "source_service_id": "a", "target_service_id": "b"},
            {"call_id": "b-to-a", "source_service_id": "b", "target_service_id": "a"},
        ],
    }


class NegativeFixtureCodeMatch(unittest.TestCase):
    def test_expected_code_present_passes(self):
        ok, _ = chk.negative_fixture_verdict("duplicate_service_id", {"duplicate_service_id", "missing_field"})
        self.assertTrue(ok)

    def test_only_unrelated_error_fails(self):
        ok, reason = chk.negative_fixture_verdict("duplicate_service_id", {"missing_field"})
        self.assertFalse(ok)
        self.assertIn("missing_field", reason)

    def test_no_findings_fails(self):
        ok, reason = chk.negative_fixture_verdict("duplicate_service_id", set())
        self.assertFalse(ok)
        self.assertIn("unexpectedly passed", reason)

    def test_missing_expected_code_field_fails(self):
        ok, reason = chk.negative_fixture_verdict(None, {"duplicate_service_id"})
        self.assertFalse(ok)
        self.assertIn("missing", reason)

    def test_unknown_expected_code_fails(self):
        ok, reason = chk.negative_fixture_verdict("not_a_real_code", {"not_a_real_code"})
        self.assertFalse(ok)
        self.assertIn("not a known emittable code", reason)

    def test_every_real_negative_fixture_asserts_its_code(self):
        neg_dir = chk.NEG_DIR
        self.assertTrue(neg_dir.exists())
        for path in sorted(neg_dir.glob("*.json")):
            obj = chk.json.loads(path.read_text(encoding="utf-8"))
            want = (obj.get("x_expect_reject") or {}).get("code")
            self.assertIsNotNone(want, f"{path.name} is missing x_expect_reject.code")
            self.assertIn(want, chk.EMITTABLE_CODES, f"{path.name} declares an unemittable code '{want}'")


class DGraphCycleAcceptance(unittest.TestCase):
    def test_two_node_cycle_is_accepted(self):
        self.assertEqual(chk.v_canonical_dgraph(sample_dgraph()), [])

    def test_self_call_is_accepted(self):
        g = sample_dgraph()
        g["calls"] = [{"call_id": "self", "source_service_id": "a", "target_service_id": "a"}]
        self.assertEqual(chk.v_canonical_dgraph(g), [])

    def test_dangling_call_target_is_the_only_thing_rejected_not_the_cycle(self):
        g = sample_dgraph()
        g["calls"][0]["target_service_id"] = "does-not-exist"
        found = codes(chk.v_canonical_dgraph(g))
        self.assertIn("dangling_call_target", found)
        self.assertNotIn("dangling_call_source", found)


class DuplicateDetectionHiddenByOrdering(unittest.TestCase):
    def test_duplicate_service_id_at_non_adjacent_positions(self):
        g = sample_dgraph()
        g["services"].append({"service_id": "a", "execution_requirement": {"dnode_abi": chk.DNODE_ABI_V0}})
        self.assertIn("duplicate_service_id", codes(chk.v_canonical_dgraph(g)))

    def test_duplicate_dnode_abi_capability_rejected(self):
        # Stage 110 P1 fix (independent review v2, re-verified v4): a
        # repeated valid ABI token must be rejected -- dnode_abi_versions is
        # a documented set, and a repeated valid value is still invalid
        # input, distinct from an unrecognised value.
        c = {
            "schema": chk.CANONICAL_DCONTINUUM_SCHEMA,
            "project_id": "p", "dcontinuum_id": "continuum:x", "revision": 1,
            "nodes": [{
                "node_id": "n1",
                "platform": {"os": "linux", "architecture": "amd64"},
                "capabilities": {"dnode_abi_versions": [chk.DNODE_ABI_V0, chk.DNODE_ABI_V0]},
            }],
            "links": [],
        }
        found = codes(chk.v_canonical_dcontinuum(c))
        self.assertIn("duplicate_dnode_abi_capability", found)

    def test_unique_dnode_abi_capabilities_reordered_remain_valid(self):
        c = {
            "schema": chk.CANONICAL_DCONTINUUM_SCHEMA,
            "project_id": "p", "dcontinuum_id": "continuum:x", "revision": 1,
            "nodes": [{
                "node_id": "n1",
                "platform": {"os": "linux", "architecture": "amd64"},
                "capacity": {"cpu_cores": 4, "memory_bytes": 4294967296},
                "capabilities": {"dnode_abi_versions": [chk.DNODE_ABI_V0]},
            }],
            "links": [],
        }
        self.assertEqual(chk.v_canonical_dcontinuum(c), [])


class DigestOrderIndependenceAndWhitelist(unittest.TestCase):
    def test_dgraph_digest_is_order_independent(self):
        g = sample_dgraph()
        reordered = dict(g)
        reordered["services"] = list(reversed(g["services"]))
        reordered["calls"] = list(reversed(g["calls"]))
        self.assertEqual(chk.digest_of_dgraph(g), chk.digest_of_dgraph(reordered))

    def test_dgraph_digest_changes_on_semantic_change(self):
        g = sample_dgraph()
        changed = chk.json.loads(chk.json.dumps(g))
        changed["services"][0]["execution_requirement"]["dnode_abi"] = "datum-dnode/999"
        self.assertNotEqual(chk.digest_of_dgraph(g), chk.digest_of_dgraph(changed))

    def test_digest_ignores_bookkeeping_fields(self):
        # Proves the whitelist actually works: adding x_note/expected_digest_sha256/
        # x_expect_reject must NEVER change the recomputed digest, since those
        # fields have no matching Rust struct field and are silently dropped
        # by serde at deserialization time.
        g = sample_dgraph()
        with_bookkeeping = dict(g)
        with_bookkeeping["x_note"] = "some explanatory prose"
        with_bookkeeping["expected_digest_sha256"] = "sha256:" + "0" * 64
        with_bookkeeping["x_expect_reject"] = {"code": "schema_invalid"}
        self.assertEqual(chk.digest_of_dgraph(g), chk.digest_of_dgraph(with_bookkeeping))

    def test_dcontinuum_digest_is_order_independent_including_capabilities(self):
        c = {
            "schema": chk.CANONICAL_DCONTINUUM_SCHEMA,
            "project_id": "p",
            "dcontinuum_id": "continuum:x",
            "revision": 1,
            "nodes": [
                {"node_id": "n1", "platform": {"os": "linux", "architecture": "amd64"}, "capabilities": {"dnode_abi_versions": [chk.DNODE_ABI_V0]}},
                {"node_id": "n2", "platform": {"os": "linux", "architecture": "arm64"}, "capabilities": {"dnode_abi_versions": []}},
            ],
            "links": [{"link_id": "l1", "from_node_id": "n1", "to_node_id": "n2"}],
        }
        reordered = chk.json.loads(chk.json.dumps(c))
        reordered["nodes"] = list(reversed(reordered["nodes"]))
        self.assertEqual(chk.digest_of_dcontinuum(c), chk.digest_of_dcontinuum(reordered))

    def test_dmap_digest_is_order_independent_for_placements(self):
        m = {
            "schema": chk.CANONICAL_DMAP_SCHEMA,
            "project_id": "p", "application_id": "app:x", "dmap_id": "dmap:x", "revision": 1,
            "dgraph_ref": {"id": "dgraph:x", "revision": 1, "digest": "sha256:" + "a" * 64},
            "dcontinuum_ref": {"id": "continuum:x", "revision": 1, "digest": "sha256:" + "b" * 64},
            "placements": [{"service_id": "a", "node_id": "n1"}, {"service_id": "b", "node_id": "n2"}],
        }
        reordered = chk.json.loads(chk.json.dumps(m))
        reordered["placements"] = list(reversed(reordered["placements"]))
        self.assertEqual(chk.digest_of_dmap(m), chk.digest_of_dmap(reordered))


class CompleteDMapValidationIsFailClosed(unittest.TestCase):
    """Stage 110 P1 fix (independent review v2): `dispatch_findings`'s dmap
    branch must aggregate the referenced D-Graph/D-Continuum's OWN
    structural findings, not just the cross-reference comparison -- mirrors
    the identical Rust `CanonicalDMapV1::validate_against` fix."""

    def test_matching_but_mutually_invalid_abi_strings_are_still_rejected(self):
        dgraph = {
            "schema": chk.CANONICAL_DGRAPH_SCHEMA,
            "application_id": "app:x", "dgraph_id": "dgraph:x", "revision": 1,
            "services": [{"service_id": "a", "execution_requirement": {"dnode_abi": "datum-dnode/999"}}],
            "calls": [],
        }
        dcontinuum = {
            "schema": chk.CANONICAL_DCONTINUUM_SCHEMA,
            "project_id": "p", "dcontinuum_id": "continuum:x", "revision": 1,
            "nodes": [{"node_id": "n1", "platform": {"os": "linux", "architecture": "amd64"},
                       "capabilities": {"dnode_abi_versions": ["datum-dnode/999"]}}],
            "links": [],
        }
        dmap = {
            "schema": chk.CANONICAL_DMAP_SCHEMA,
            "project_id": "p", "application_id": "app:x", "dmap_id": "dmap:x", "revision": 1,
            "dgraph_ref": {"id": "dgraph:x", "revision": 1, "digest": chk.digest_of_dgraph(dgraph)},
            "dcontinuum_ref": {"id": "continuum:x", "revision": 1, "digest": chk.digest_of_dcontinuum(dcontinuum)},
            "placements": [{"service_id": "a", "node_id": "n1"}],
        }

        # The partial cross-only check alone sees nothing wrong (documents
        # the gap, does not endorse using it as an acceptance gate).
        cross_only = chk.v_canonical_dmap_cross(dmap, dgraph, dcontinuum)
        self.assertEqual(cross_only, [], "documents the exact gap: raw-string comparison alone sees nothing wrong")

        # The COMPLETE aggregation `dispatch_findings`'s dmap branch performs
        # -- exercised directly here (rather than through dispatch_findings,
        # which deliberately checks against the ONE real fixture-directory
        # dgraph/dcontinuum by fixed natural key, not an arbitrary caller-
        # supplied pair) since this test's synthetic objects use their own
        # identities.
        complete = (
            chk.v_canonical_dmap_structural(dmap)
            + chk.v_canonical_dgraph(dgraph)
            + chk.v_canonical_dcontinuum(dcontinuum)
            + chk.v_canonical_dmap_cross(dmap, dgraph, dcontinuum)
        )
        found = codes(complete)
        self.assertIn("invalid_dnode_abi", found, f"findings: {complete}")
        self.assertIn("invalid_dnode_abi_capability", found, f"findings: {complete}")


class ResourceFeasibilityValidation(unittest.TestCase):
    """Stage 110 P0 fix (independent review v3, Stage 112 resource
    readiness): mirrors the identical Rust regression tests."""

    def _dgraph_with_resources(self, cpu_a=1, mem_a=536870912, cpu_b=1, mem_b=268435456):
        g = sample_dgraph()
        g["services"][0]["resource_requirement"] = {"cpu_cores": cpu_a, "memory_bytes": mem_a}
        g["services"][1]["resource_requirement"] = {"cpu_cores": cpu_b, "memory_bytes": mem_b}
        return g

    def _dcontinuum_single_node(self, cpu=4, mem=4294967296):
        return {
            "schema": chk.CANONICAL_DCONTINUUM_SCHEMA,
            "project_id": "p", "dcontinuum_id": "continuum:x", "revision": 1,
            "nodes": [{
                "node_id": "n1",
                "platform": {"os": "linux", "architecture": "amd64"},
                "capacity": {"cpu_cores": cpu, "memory_bytes": mem},
                "capabilities": {"dnode_abi_versions": [chk.DNODE_ABI_V0]},
            }],
            "links": [],
        }

    def _dmap_both_on_n1(self, dgraph, dcontinuum):
        return {
            "schema": chk.CANONICAL_DMAP_SCHEMA,
            "project_id": "p", "application_id": "app:x", "dmap_id": "dmap:x", "revision": 1,
            "dgraph_ref": {"id": "dgraph:x", "revision": 1, "digest": chk.digest_of_dgraph(dgraph)},
            "dcontinuum_ref": {"id": "continuum:x", "revision": 1, "digest": chk.digest_of_dcontinuum(dcontinuum)},
            "placements": [{"service_id": "a", "node_id": "n1"}, {"service_id": "b", "node_id": "n1"}],
        }

    def test_invalid_cpu_requirement_rejected(self):
        g = self._dgraph_with_resources(cpu_a=0)
        self.assertIn("invalid_cpu_requirement", codes(chk.v_canonical_dgraph(g)))

    def test_invalid_memory_requirement_rejected(self):
        g = self._dgraph_with_resources(mem_a=0)
        self.assertIn("invalid_memory_requirement", codes(chk.v_canonical_dgraph(g)))

    def test_invalid_node_cpu_capacity_rejected(self):
        c = self._dcontinuum_single_node(cpu=0)
        self.assertIn("invalid_node_cpu_capacity", codes(chk.v_canonical_dcontinuum(c)))

    def test_invalid_node_memory_capacity_rejected(self):
        c = self._dcontinuum_single_node(mem=0)
        self.assertIn("invalid_node_memory_capacity", codes(chk.v_canonical_dcontinuum(c)))

    def test_multiple_services_within_aggregate_capacity_remain_valid(self):
        dgraph = self._dgraph_with_resources()
        dcontinuum = self._dcontinuum_single_node()
        dmap = self._dmap_both_on_n1(dgraph, dcontinuum)
        found = codes(chk.v_canonical_dmap_cross(dmap, dgraph, dcontinuum))
        self.assertFalse(any("cpu" in c or "memory" in c for c in found), f"found: {found}")

    def test_service_cpu_requirement_larger_than_node_capacity_rejected(self):
        dgraph = self._dgraph_with_resources(cpu_a=100)
        dcontinuum = self._dcontinuum_single_node()
        dmap = self._dmap_both_on_n1(dgraph, dcontinuum)
        self.assertIn("insufficient_node_cpu", codes(chk.v_canonical_dmap_cross(dmap, dgraph, dcontinuum)))

    def test_service_memory_requirement_larger_than_node_capacity_rejected(self):
        dgraph = self._dgraph_with_resources(mem_b=8 * 1024 * 1024 * 1024)
        dcontinuum = self._dcontinuum_single_node()
        dmap = self._dmap_both_on_n1(dgraph, dcontinuum)
        self.assertIn("insufficient_node_memory", codes(chk.v_canonical_dmap_cross(dmap, dgraph, dcontinuum)))

    def test_aggregate_cpu_overcommit_rejected_even_when_each_service_individually_fits(self):
        dgraph = self._dgraph_with_resources(cpu_a=3, cpu_b=3)  # each <=4, combined 6 > 4
        dcontinuum = self._dcontinuum_single_node()
        dmap = self._dmap_both_on_n1(dgraph, dcontinuum)
        found = codes(chk.v_canonical_dmap_cross(dmap, dgraph, dcontinuum))
        self.assertNotIn("insufficient_node_cpu", found, f"individual fit must not fire: {found}")
        self.assertIn("aggregate_node_cpu_exceeded", found)

    def test_aggregate_memory_overcommit_rejected_even_when_each_service_individually_fits(self):
        dgraph = self._dgraph_with_resources(mem_a=3221225472, mem_b=3221225472)  # each <=4GiB, combined 6GiB > 4GiB
        dcontinuum = self._dcontinuum_single_node()
        dmap = self._dmap_both_on_n1(dgraph, dcontinuum)
        found = codes(chk.v_canonical_dmap_cross(dmap, dgraph, dcontinuum))
        self.assertNotIn("insufficient_node_memory", found, f"individual fit must not fire: {found}")
        self.assertIn("aggregate_node_memory_exceeded", found)

    def test_no_live_resource_usage_field_exists_in_the_digest_whitelist(self):
        # PASS P (independent review v3): the whitelist functions only ever
        # copy `cpu_cores`/`memory_bytes` -- never a usage/available/current
        # field, since no such field exists on the Rust types this whitelist
        # mirrors.
        dgraph = self._dgraph_with_resources()
        digest_text = chk.canonicalize_json_text(
            {"services": [{"resource_requirement": {"cpu_cores": s["resource_requirement"]["cpu_cores"], "memory_bytes": s["resource_requirement"]["memory_bytes"]}} for s in dgraph["services"]]}
        )
        for forbidden in ["cpu_usage", "usage_percent", "available", "current_", "rss", "load_average"]:
            self.assertNotIn(forbidden, digest_text)


class ReferenceDigestFormatValidation(unittest.TestCase):
    """Stage 110 P1 fix (independent review v3): `dgraph_ref.digest`/
    `dcontinuum_ref.digest` must be the exact ADR-0006 canonical form."""

    def _dmap(self, dgraph_digest, dcontinuum_digest):
        return {
            "schema": chk.CANONICAL_DMAP_SCHEMA,
            "project_id": "p", "application_id": "app:x", "dmap_id": "dmap:x", "revision": 1,
            "dgraph_ref": {"id": "dgraph:x", "revision": 1, "digest": dgraph_digest},
            "dcontinuum_ref": {"id": "continuum:x", "revision": 1, "digest": dcontinuum_digest},
            "placements": [],
        }

    def test_empty_digest_rejected(self):
        m = self._dmap("", "sha256:" + "b" * 64)
        self.assertIn("invalid_digest_format", codes(chk.v_canonical_dmap_structural(m)))

    def test_bare_hex_digest_rejected(self):
        m = self._dmap("a" * 64, "sha256:" + "b" * 64)
        self.assertIn("invalid_digest_format", codes(chk.v_canonical_dmap_structural(m)))

    def test_uppercase_hex_digest_rejected(self):
        m = self._dmap("sha256:" + "A" * 64, "sha256:" + "b" * 64)
        self.assertIn("invalid_digest_format", codes(chk.v_canonical_dmap_structural(m)))

    def test_wrong_length_digest_rejected(self):
        m = self._dmap("sha256:" + "a" * 63, "sha256:" + "b" * 64)
        self.assertIn("invalid_digest_format", codes(chk.v_canonical_dmap_structural(m)))

    def test_well_formed_digest_accepted_by_format_check(self):
        m = self._dmap("sha256:" + "a" * 64, "sha256:" + "b" * 64)
        self.assertNotIn("invalid_digest_format", codes(chk.v_canonical_dmap_structural(m)))

    def test_is_canonical_digest_format_matches_rust_semantics(self):
        self.assertTrue(chk.is_canonical_digest_format("sha256:" + "a" * 64))
        self.assertFalse(chk.is_canonical_digest_format("a" * 64))
        self.assertFalse(chk.is_canonical_digest_format("sha256:" + "a" * 63))
        self.assertFalse(chk.is_canonical_digest_format("sha256:" + "a" * 65))
        self.assertFalse(chk.is_canonical_digest_format("sha256:" + "A" * 64))
        self.assertFalse(chk.is_canonical_digest_format(""))
        self.assertFalse(chk.is_canonical_digest_format(None))


class SchemaIdUniqueness(unittest.TestCase):
    """Stage 110 P0 fix (independent review v2, widened in v3) / PASS I + O."""

    def test_real_repository_state_has_no_collision(self):
        rep = chk.Report()
        chk.check_schema_id_uniqueness(rep)
        self.assertEqual(rep.failures, [], f"failures: {rep.failures}")

    def test_a_third_fixture_family_reclaiming_a_production_schema_id_is_caught(self):
        # PASS O (independent review v3): the v2 version of this check only
        # ever scanned two NAMED directories (`canonical-v1/` and `vs1/`) --
        # a third fixture family added later under
        # `scripts/fixtures/datum-contracts/<something-else>/` would have
        # gone completely unscanned. This proof builds a synthetic tree with
        # a `canonical-v1/` subdirectory (legitimate) AND a THIRD, arbitrary
        # sibling directory that reclaims a production schema ID, and
        # confirms the (now directory-name-agnostic) checker still catches
        # it by walking the whole tree and checking each file's actual
        # `schema` field.
        import json
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            canonical_v1 = tmp_path / "canonical-v1"
            canonical_v1.mkdir()
            (canonical_v1 / "dgraph.ok.json").write_text(
                json.dumps({"schema": "datum.dgraph/1"}), encoding="utf-8")
            (canonical_v1 / "dcontinuum.ok.json").write_text(
                json.dumps({"schema": "datum.dcontinuum/1"}), encoding="utf-8")
            (canonical_v1 / "dmap.ok.json").write_text(
                json.dumps({"schema": "datum.dmap/1"}), encoding="utf-8")
            third_family = tmp_path / "some-other-sketch-family"
            third_family.mkdir()
            (third_family / "dgraph.regressed.json").write_text(
                json.dumps({"schema": "datum.dgraph/1", "dgraph_id": "dgraph:regressed", "revision": 1}),
                encoding="utf-8",
            )
            original_root, original_fixture_dir = chk.DATUM_CONTRACTS_ROOT, chk.FIXTURE_DIR
            try:
                chk.DATUM_CONTRACTS_ROOT = tmp_path
                chk.FIXTURE_DIR = canonical_v1
                rep = chk.Report()
                chk.check_schema_id_uniqueness(rep)
            finally:
                chk.DATUM_CONTRACTS_ROOT, chk.FIXTURE_DIR = original_root, original_fixture_dir

            self.assertTrue(
                any("declares production schema 'datum.dgraph/1'" in f and "some-other-sketch-family" in f for f in rep.failures),
                f"expected a schema-id-uniqueness failure naming the third family; got: {rep.failures}",
            )

    def test_missing_production_schema_entirely_is_also_caught(self):
        # The check must not trivially pass by finding nothing at all --
        # if canonical-v1/ stopped declaring one of the three production
        # IDs (e.g. an accidental rename), that is itself a failure.
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            empty_dir = Path(tmp)
            original_root = chk.DATUM_CONTRACTS_ROOT
            try:
                chk.DATUM_CONTRACTS_ROOT = empty_dir
                rep = chk.Report()
                chk.check_schema_id_uniqueness(rep)
            finally:
                chk.DATUM_CONTRACTS_ROOT = original_root

            self.assertTrue(
                any("not found in any canonical-v1 fixture" in f for f in rep.failures),
                f"expected a missing-production-schema failure; got: {rep.failures}",
            )


class RealFixturesEndToEnd(unittest.TestCase):
    """A full re-run of the main checker's own logic against the real
    fixture directory -- proves the selftest module and the checker module
    stay in agreement, and gives one place that fails loudly if the fixture
    directory itself becomes inconsistent."""

    def test_main_checker_passes_against_the_real_fixture_directory(self):
        rep = chk.Report()
        chk.check_fixtures(rep)
        self.assertEqual(rep.failures, [], f"failures: {rep.failures}")


if __name__ == "__main__":
    unittest.main()
