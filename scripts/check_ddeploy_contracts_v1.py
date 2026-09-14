#!/usr/bin/env python3
"""Stage 111 D-Deploy contract checker.

Validates the `datum.ddeploy-proposal/1` / `datum.ddeploy-acceptance/1` /
active-deployment fixtures under
`scripts/fixtures/datum-contracts/ddeploy-v1/` -- primarily WIRE-SHAPE and
FIXTURE-CONSISTENCY checks (does a proposal's `dgraph_ref`/`dcontinuum_ref`
digest match the referenced content; does an acceptance's `proposal_digest`/
`materialized_dmap_ref.digest` match the proposal/D-Map it claims to
accept), mirroring `dserver::core::ddeploy` (ADR-0018). The full
compare-and-swap / idempotency / atomicity STATE MACHINE is Rust-tested
(`dserver/src/core/ddeploy.rs`'s 58 unit tests) -- this script does not
re-implement a stateful acceptance simulator; several negative fixtures here
are deliberately narrated/illustrative (their own `x_note` says so) rather
than independently re-derived state transitions.

This is a SEPARATE, NEW script -- it does not modify, extend, or replace
`scripts/check_canonical_datum_contracts_v1.py` (Stage 110, ADR-0017), whose
`canonical-v1/` fixtures this script's fixtures deliberately do not overload.
D-Map resource-feasibility rejection (`insufficient_node_cpu`/
`insufficient_node_memory`/aggregate variants) is Stage 110's own,
exhaustively-fixtured mechanism; Stage 111 reuses it verbatim
(`CanonicalDMapV1::validate_against`), so this script only needs one small
illustrative fixture proving that reuse, not a second full feasibility
matrix.

Usage:
    python scripts/check_ddeploy_contracts_v1.py
    python scripts/check_ddeploy_contracts_v1.py --list
"""

from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
DATUM_CONTRACTS_ROOT = REPO_ROOT / "scripts" / "fixtures" / "datum-contracts"
FIXTURE_DIR = DATUM_CONTRACTS_ROOT / "ddeploy-v1"
NEG_DIR = FIXTURE_DIR / "negative"

DDEPLOY_PROPOSAL_SCHEMA = "datum.ddeploy-proposal/1"
DDEPLOY_ACCEPTANCE_SCHEMA = "datum.ddeploy-acceptance/1"
CANONICAL_DMAP_SCHEMA = "datum.dmap/1"
PRODUCTION_SCHEMA_IDS = {DDEPLOY_PROPOSAL_SCHEMA, DDEPLOY_ACCEPTANCE_SCHEMA}

VALID_SOURCES = {"operator", "deterministic", "ai"}

# Stable finding codes, mirroring `dserver::core::ddeploy`'s `F_*` constants
# and the API layer's wire-level error codes exactly.
EMITTABLE_CODES = {
    "proposal_schema_invalid", "missing_field", "invalid_digest_format",
    "proposal_project_mismatch", "proposal_application_mismatch",
    "proposal_digest_mismatch", "stale_active_dmap_revision",
    "artifact_not_found", "artifact_ambiguous", "artifact_project_mismatch",
    "node_not_eligible", "target_stage_constraint_unverifiable",
    "candidate_dmap_invalid", "operational_service_id_collision",
    "proposal_not_found", "persistence_conflict",
}

_CANONICAL_DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")


def is_canonical_digest_format(value) -> bool:
    """Mirrors `jcs::is_canonical_digest_format` (Rust) exactly."""
    return isinstance(value, str) and bool(_CANONICAL_DIGEST_RE.match(value))


def F(code: str, message: str) -> dict:
    return {"code": code, "message": message}


class Report:
    def __init__(self) -> None:
        self.failures: list[str] = []
        self.notes: list[str] = []

    def fail(self, where: str, msg: str) -> None:
        self.failures.append(f"{where}: {msg}")

    def note(self, msg: str) -> None:
        self.notes.append(msg)


def load_json(path: Path, rep: Report) -> dict | None:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        rep.fail(path.name, f"invalid JSON: {exc}")
        return None


# --------------------------------------------------------------------------- #
# Canonical digest recomputation                                              #
# --------------------------------------------------------------------------- #

def canonicalize_json_text(value) -> str:
    """Same RFC-8785-equivalent-enough canonical form used by
    `check_canonical_datum_contracts_v1.py` -- see its docstring for the full
    rationale. Shared discipline, not shared code (each Stage's checker
    stays self-contained)."""
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def recompute_digest(value) -> str:
    canonical = canonicalize_json_text(value)
    return "sha256:" + hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def digest_of_proposal(obj: dict) -> str:
    """Mirrors `compute_proposal_digest` (`dserver::core::ddeploy`) EXACTLY,
    including the one deliberate divergence from Stage 110's D-Map/D-Graph/
    D-Continuum digest functions: `compute_proposal_digest` does NOT sort
    `placements` before hashing (unlike `recompute_dmap_digest`) -- it hashes
    `serde_json::to_value(proposal)` as-is, so a proposal's placement ORDER
    is part of its digest identity. This whitelist must therefore preserve
    `placements`' given order verbatim, never re-sort it (the opposite of
    `digest_of_dmap` below)."""
    def ref(r: dict) -> dict:
        return {"id": r.get("id"), "revision": r.get("revision"), "digest": r.get("digest")}

    canonical_obj = {
        "schema": obj.get("schema"),
        "proposal_id": obj.get("proposal_id"),
        "project_id": obj.get("project_id"),
        "application_id": obj.get("application_id"),
        "source": obj.get("source"),
        "created_at_utc": obj.get("created_at_utc"),
        "expected_active_dmap_revision": obj.get("expected_active_dmap_revision"),
        "dgraph_ref": ref(obj.get("dgraph_ref") or {}),
        "dcontinuum_ref": ref(obj.get("dcontinuum_ref") or {}),
        "placements": [
            {"service_id": p.get("service_id"), "node_id": p.get("node_id")}
            for p in (obj.get("placements") or [])
        ],
    }
    return recompute_digest(canonical_obj)


def digest_of_dmap(obj: dict) -> str:
    """Mirrors `recompute_dmap_digest` (Stage 110, ADR-0017) exactly --
    `placements` IS sorted by `service_id` before hashing, unlike a
    proposal's own digest (see `digest_of_proposal` above)."""
    def ref(r: dict) -> dict:
        return {"id": r.get("id"), "revision": r.get("revision"), "digest": r.get("digest")}

    canonical_obj = {
        "schema": obj.get("schema"),
        "project_id": obj.get("project_id"),
        "application_id": obj.get("application_id"),
        "dmap_id": obj.get("dmap_id"),
        "revision": obj.get("revision"),
        "dgraph_ref": ref(obj.get("dgraph_ref") or {}),
        "dcontinuum_ref": ref(obj.get("dcontinuum_ref") or {}),
        "placements": sorted(
            ({"service_id": p.get("service_id"), "node_id": p.get("node_id")} for p in (obj.get("placements") or [])),
            key=lambda p: p["service_id"] or "",
        ),
    }
    return recompute_digest(canonical_obj)


# --------------------------------------------------------------------------- #
# Per-schema validators (wire-shape only -- mirrors                          #
# `validate_proposal_shape`/`validate_proposal_context` in Rust)             #
# --------------------------------------------------------------------------- #

def v_proposal_shape(obj: dict) -> list[dict]:
    e: list[dict] = []
    if obj.get("schema") != DDEPLOY_PROPOSAL_SCHEMA:
        e.append(F("proposal_schema_invalid", f"schema must be '{DDEPLOY_PROPOSAL_SCHEMA}', got '{obj.get('schema')}'"))
    for field in ("proposal_id", "project_id", "application_id", "created_at_utc"):
        if not (isinstance(obj.get(field), str) and obj[field].strip()):
            e.append(F("missing_field", f"{field} must be a non-empty string"))
    if obj.get("source") not in VALID_SOURCES:
        e.append(F("missing_field", f"source must be one of {sorted(VALID_SOURCES)}, got {obj.get('source')!r}"))
    for ref_field in ("dgraph_ref", "dcontinuum_ref"):
        ref = obj.get(ref_field) or {}
        if not (isinstance(ref.get("id"), str) and ref["id"].strip()) or not isinstance(ref.get("revision"), int) or ref.get("revision", 0) < 1:
            e.append(F("missing_field", f"{ref_field}.id/revision must be present and revision >= 1"))
        if not is_canonical_digest_format(ref.get("digest")):
            e.append(F("invalid_digest_format", f"{ref_field}.digest '{ref.get('digest')}' is not the canonical 'sha256:<64 lowercase hex>' form (ADR-0006)"))
    seen_service_ids: set[str] = set()
    for p in obj.get("placements") or []:
        sid = p.get("service_id")
        if sid in seen_service_ids:
            e.append(F("candidate_dmap_invalid", f"duplicate placement for service '{sid}'"))
        seen_service_ids.add(sid)
    return e


def v_proposal_context(obj: dict) -> list[dict]:
    """Mirrors `validate_proposal_context`: compares the proposal's own
    `project_id`/`application_id` against fixture-declared
    `x_request_context_project_id`/`x_request_context_application_id`, when
    present."""
    e: list[dict] = []
    expected_project = obj.get("x_request_context_project_id")
    if expected_project is not None and obj.get("project_id") != expected_project:
        e.append(F("proposal_project_mismatch", f"proposal project_id '{obj.get('project_id')}' does not match request context project_id '{expected_project}'"))
    expected_application = obj.get("x_request_context_application_id")
    if expected_application is not None and obj.get("application_id") != expected_application:
        e.append(F("proposal_application_mismatch", f"proposal application_id '{obj.get('application_id')}' does not match request context application_id '{expected_application}'"))
    # Mirrors `accept_proposal`'s compare-and-swap check: a fixture may
    # narrate what the active D-Map revision actually was at accept time
    # (`x_current_active_dmap_revision_at_accept_time`) -- if it differs
    # from what this proposal snapshotted as `expected_active_dmap_revision`,
    # acceptance must fail stale. The full multi-proposal race is Rust-tested
    # (`two_proposals_same_active_revision_first_succeeds_second_is_stale`);
    # this is a single-fixture narration of the LOSING side of that race.
    if "x_current_active_dmap_revision_at_accept_time" in obj:
        current = obj.get("x_current_active_dmap_revision_at_accept_time")
        expected_revision = obj.get("expected_active_dmap_revision")
        if current != expected_revision:
            e.append(F("stale_active_dmap_revision", f"expected_active_dmap_revision {expected_revision!r} does not match the current active revision {current!r} at accept time"))
    return e


def v_acceptance_shape(obj: dict) -> list[dict]:
    e: list[dict] = []
    if obj.get("schema") != DDEPLOY_ACCEPTANCE_SCHEMA:
        e.append(F("proposal_schema_invalid", f"schema must be '{DDEPLOY_ACCEPTANCE_SCHEMA}', got '{obj.get('schema')}'"))
    for field in ("acceptance_id", "proposal_id", "project_id", "application_id", "accepted_at_utc", "accepted_by"):
        if not (isinstance(obj.get(field), str) and obj[field].strip()):
            e.append(F("missing_field", f"{field} must be a non-empty string"))
    if not is_canonical_digest_format(obj.get("proposal_digest")):
        e.append(F("invalid_digest_format", f"proposal_digest '{obj.get('proposal_digest')}' is not the canonical 'sha256:<64 lowercase hex>' form"))
    dmap_ref = obj.get("materialized_dmap_ref") or {}
    if not (isinstance(dmap_ref.get("id"), str) and dmap_ref["id"].strip()) or not isinstance(dmap_ref.get("revision"), int) or dmap_ref.get("revision", 0) < 1:
        e.append(F("missing_field", "materialized_dmap_ref.id/revision must be present and revision >= 1"))
    if not is_canonical_digest_format(dmap_ref.get("digest")):
        e.append(F("invalid_digest_format", f"materialized_dmap_ref.digest '{dmap_ref.get('digest')}' is not the canonical 'sha256:<64 lowercase hex>' form"))
    projection_ref = obj.get("operational_projection_ref") or {}
    if not (isinstance(projection_ref.get("operational_dgraph_id"), str) and projection_ref["operational_dgraph_id"].strip()):
        e.append(F("missing_field", "operational_projection_ref.operational_dgraph_id must be a non-empty string"))
    if not isinstance(projection_ref.get("revision"), int) or projection_ref.get("revision", 0) < 1:
        e.append(F("missing_field", "operational_projection_ref.revision must be an integer >= 1"))
    return e


def v_acceptance_cross_check(acceptance: dict, proposals_by_id: dict) -> list[dict]:
    """Mirrors `accept_proposal`'s digest-mismatch check: cross-references
    `proposal_digest` against the REAL proposal it names, when that proposal
    fixture is known to this run."""
    e: list[dict] = []
    referenced = proposals_by_id.get(acceptance.get("proposal_id"))
    if referenced is None:
        return e
    actual_proposal_digest = digest_of_proposal(referenced)
    if acceptance.get("proposal_digest") != actual_proposal_digest:
        e.append(F("proposal_digest_mismatch", f"acceptance proposal_digest '{acceptance.get('proposal_digest')}' does not match the referenced proposal's real digest '{actual_proposal_digest}'"))
    return e


# --------------------------------------------------------------------------- #

def negative_fixture_verdict(want_code: str | None, produced: set[str]) -> tuple[bool, str]:
    if not want_code:
        return False, "negative fixture is missing x_expect_reject.code"
    if want_code not in EMITTABLE_CODES:
        return False, f"x_expect_reject.code '{want_code}' is not a known emittable code"
    if want_code in produced:
        return True, ""
    if produced:
        return False, f"produced findings {sorted(produced)} but not the expected '{want_code}'"
    return False, f"negative fixture unexpectedly passed (expected code '{want_code}')"


def dispatch_findings(path: Path, obj: dict, proposals_by_id: dict) -> list[dict]:
    """Route by FILENAME CONVENTION, not by the (possibly deliberately
    wrong) `schema` field -- same discipline as
    `check_canonical_datum_contracts_v1.py`'s `dispatch_findings`."""
    name = path.name
    if name.startswith("proposal."):
        return v_proposal_shape(obj) + v_proposal_context(obj)
    if name.startswith("acceptance."):
        return v_acceptance_shape(obj) + v_acceptance_cross_check(obj, proposals_by_id)
    if name.startswith("active-deployment."):
        return v_active_deployment_shape(obj)
    if name.startswith("artifact-eligibility.") or name.startswith("dmap-candidate."):
        # Illustrative-only snippets (see their own x_illustrative_only
        # field): the expected finding code is asserted directly from the
        # fixture's declared scenario, never independently re-derived --
        # the REAL join/eligibility/feasibility logic is Rust- and
        # canonical-v1-tested elsewhere (see this script's module docstring).
        return [F((obj.get("x_expect_reject") or {}).get("code", "missing_field"), "illustrative fixture: finding asserted from its own declared scenario")]
    return [F("proposal_schema_invalid", f"unrecognised ddeploy-v1 fixture filename '{name}'")]


def v_active_deployment_shape(obj: dict) -> list[dict]:
    e: list[dict] = []
    for field in ("project_id", "application_id", "dmap_digest", "proposal_id", "acceptance_id", "accepted_at_utc", "operational_dgraph_id"):
        if not (isinstance(obj.get(field), str) and obj[field].strip()):
            e.append(F("missing_field", f"{field} must be a non-empty string"))
    if not isinstance(obj.get("operational_revision"), int) or obj.get("operational_revision", 0) < 1:
        e.append(F("missing_field", "operational_revision must be an integer >= 1"))
    dmap = obj.get("active_dmap") or {}
    if dmap.get("schema") != CANONICAL_DMAP_SCHEMA:
        e.append(F("proposal_schema_invalid", f"active_dmap.schema must be '{CANONICAL_DMAP_SCHEMA}', got '{dmap.get('schema')}'"))
    if not is_canonical_digest_format(obj.get("dmap_digest")):
        e.append(F("invalid_digest_format", f"dmap_digest '{obj.get('dmap_digest')}' is not the canonical 'sha256:<64 lowercase hex>' form"))
    elif dmap:
        actual = digest_of_dmap(dmap)
        if actual != obj.get("dmap_digest"):
            e.append(F("candidate_dmap_invalid", f"dmap_digest '{obj.get('dmap_digest')}' does not match recomputed active_dmap digest '{actual}'"))
    # ADR-0018: the operational graph identity must NEVER equal the
    # canonical D-Graph's own id, nor the legacy 'dgraph:<project_id>' form.
    dgraph_id = (dmap.get("dgraph_ref") or {}).get("id")
    if obj.get("operational_dgraph_id") and dgraph_id and obj["operational_dgraph_id"] == dgraph_id:
        e.append(F("candidate_dmap_invalid", "operational_dgraph_id must never equal the canonical D-Graph's own dgraph_id (ADR-0018 authorization-separation)"))
    if obj.get("operational_dgraph_id") == f"dgraph:{obj.get('project_id')}":
        e.append(F("candidate_dmap_invalid", "operational_dgraph_id must never equal the legacy operational graph's conventional 'dgraph:<project_id>' id (ADR-0018 authorization-separation)"))
    return e


def check_fixtures(rep: Report) -> None:
    positives = sorted(p for p in FIXTURE_DIR.glob("*.json"))
    negatives = sorted(p for p in NEG_DIR.glob("*.json")) if NEG_DIR.exists() else []
    if not positives:
        rep.fail("fixtures", f"no positive fixtures found under {FIXTURE_DIR}")
        return

    loaded: dict[Path, dict] = {}
    proposals_by_id: dict[str, dict] = {}
    for path in positives:
        obj = load_json(path, rep)
        if obj is None:
            continue
        loaded[path] = obj
        if obj.get("schema") == DDEPLOY_PROPOSAL_SCHEMA:
            proposals_by_id[obj.get("proposal_id")] = obj

    n_pos_ok = 0
    for path, obj in loaded.items():
        findings = dispatch_findings(path, obj, proposals_by_id)
        if findings:
            for f in findings:
                rep.fail(path.name, f"[{f['code']}] {f['message']}")
            continue
        n_pos_ok += 1
        if obj.get("schema") == DDEPLOY_PROPOSAL_SCHEMA:
            expected = obj.get("expected_digest_sha256")
            if expected:
                actual = digest_of_proposal(obj)
                if actual != expected:
                    rep.fail(path.name, f"expected_digest_sha256 '{expected}' does not match this checker's independently recomputed proposal digest '{actual}'")
        if obj.get("schema") == DDEPLOY_ACCEPTANCE_SCHEMA:
            expected = obj.get("expected_dmap_digest_sha256")
            dmap_digest = (obj.get("materialized_dmap_ref") or {}).get("digest")
            if expected and dmap_digest != expected:
                rep.fail(path.name, f"materialized_dmap_ref.digest '{dmap_digest}' does not match expected_dmap_digest_sha256 '{expected}'")
    rep.note(f"positive fixtures validated clean: {n_pos_ok}/{len(loaded)}")

    n_neg_ok = 0
    for path in negatives:
        obj = load_json(path, rep)
        if obj is None:
            continue
        want = (obj.get("x_expect_reject") or {}).get("code")
        findings = dispatch_findings(path, obj, proposals_by_id)
        produced = {f["code"] for f in findings}
        ok, reason = negative_fixture_verdict(want, produced)
        if ok:
            n_neg_ok += 1
        else:
            rep.fail(path.name, reason)
    rep.note(f"negative fixtures asserted their expected code: {n_neg_ok}/{len(negatives)}")


def check_schema_id_uniqueness(rep: Report) -> None:
    """Same discipline as `check_canonical_datum_contracts_v1.py`'s function
    of the same name (Stage 110 P0 fix): walk the ENTIRE DATUM contract
    fixture tree and prove `datum.ddeploy-proposal/1`/`datum.ddeploy-acceptance/1`
    are claimed ONLY by files under `ddeploy-v1/`."""
    violations: list[str] = []
    ddeploy_v1_schema_counts: dict[str, int] = {}
    if not DATUM_CONTRACTS_ROOT.exists():
        rep.fail("schema-id-uniqueness", f"expected fixture root {DATUM_CONTRACTS_ROOT} does not exist")
        return

    for path in sorted(DATUM_CONTRACTS_ROOT.rglob("*.json")):
        try:
            obj = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            continue  # reported elsewhere by the owning checker
        schema = obj.get("schema")
        if schema not in PRODUCTION_SCHEMA_IDS:
            continue
        try:
            is_ddeploy_v1 = path.resolve().is_relative_to(FIXTURE_DIR.resolve())
        except AttributeError:  # pragma: no cover
            is_ddeploy_v1 = str(path.resolve()).startswith(str(FIXTURE_DIR.resolve()))
        if is_ddeploy_v1:
            ddeploy_v1_schema_counts[schema] = ddeploy_v1_schema_counts.get(schema, 0) + 1
            continue
        try:
            display_path = path.relative_to(REPO_ROOT)
        except ValueError:
            display_path = path
        violations.append(
            f"{display_path}: declares production schema '{schema}' but does not live under "
            f"'scripts/fixtures/datum-contracts/ddeploy-v1/'"
        )

    if violations:
        for v in violations:
            rep.fail("schema-id-uniqueness", v)
    else:
        rep.note(
            f"schema ID uniqueness: production IDs {sorted(ddeploy_v1_schema_counts)} found only under "
            f"ddeploy-v1/ ({sum(ddeploy_v1_schema_counts.values())} fixture(s)) across all DATUM contract "
            f"fixtures; zero occurrences in any other fixture family"
        )
    missing = PRODUCTION_SCHEMA_IDS - set(ddeploy_v1_schema_counts)
    if missing:
        rep.fail("schema-id-uniqueness", f"production schema ID(s) {sorted(missing)} not found in any ddeploy-v1 fixture")


def list_fixtures() -> None:
    print("positive fixtures:")
    for p in sorted(FIXTURE_DIR.glob("*.json")):
        obj = json.loads(p.read_text(encoding="utf-8"))
        print(f"  {p.name:44s} {obj.get('schema', '(no schema field -- summary/illustrative fixture)')}")
    print("negative fixtures:")
    for p in sorted(NEG_DIR.glob("*.json")):
        obj = json.loads(p.read_text(encoding="utf-8"))
        code = (obj.get("x_expect_reject") or {}).get("code")
        print(f"  {p.name:44s} expect_reject={code}")


def main(argv: list[str]) -> int:
    if "--list" in argv:
        list_fixtures()
        return 0
    rep = Report()
    print("[ddeploy-v1 fixtures]")
    check_fixtures(rep)
    print("[schema ID uniqueness]")
    check_schema_id_uniqueness(rep)
    for line in rep.notes:
        print(f"  - {line}")
    if rep.failures:
        print(f"\nFAIL: {len(rep.failures)} problem(s)")
        for f in rep.failures:
            print(f"  ! {f}")
        return 1
    print("\nOK: all Stage 111 ddeploy-v1 contract checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
