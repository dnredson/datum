#!/usr/bin/env python3
"""Stage 110 canonical-v1 contract checker.

Validates the canonical-v1 contract fixtures under
`scripts/fixtures/datum-contracts/canonical-v1/` -- an INDEPENDENT Python
re-implementation of the same semantic rules
`dserver::core::canonical_dgraph`/`canonical_dcontinuum`/`canonical_dmap`
and their `DATUM::agent::canonical_*` mirrors implement in Rust (ADR-0017).

This is a SEPARATE, NEW script -- it does not modify, extend, or replace
`scripts/check_datum_contract_fixtures.py`, which continues to validate the
Stage 107 hand-authored VS1 fixtures under `.../vs1/` (a materially richer,
never-compiled sketch of a future D-Deploy v0 vision: D-Compile lineage,
D-Forward/D-IoT placement, ABI/host-kind negotiation, explicit
proposal-acceptance decisions). The two fixture families intentionally use
DIFFERENT, non-overlapping directories (`vs1/` vs `canonical-v1/`) precisely
so this script's glob can never collide with the older one's, and vice versa
-- see ADR-0017's "legacy collision audit" for the full rationale.

Per-schema validators return **structured findings** `{code, message}`,
using the EXACT SAME code vocabulary as the Rust
`CanonicalValidationFinding.code` values -- deliberate parity, not
coincidence, so a finding code means the same thing in Python, dserver-Rust,
and DATUM-Rust. A positive fixture passes with zero findings. A negative
fixture (under `negative/`) declares `x_expect_reject.code` and passes only
when that exact code is among the produced finding codes.

Usage:
    python scripts/check_canonical_datum_contracts_v1.py
    python scripts/check_canonical_datum_contracts_v1.py --list
"""

from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
DATUM_CONTRACTS_ROOT = REPO_ROOT / "scripts" / "fixtures" / "datum-contracts"
FIXTURE_DIR = DATUM_CONTRACTS_ROOT / "canonical-v1"
NEG_DIR = FIXTURE_DIR / "negative"

# Stage 110 P0 fix (independent review v4, D-Serv host invariance): the ONLY
# accepted D-Node ABI for v0.1 (ADR-0003). Deliberately NOT Stage 108's
# RuntimeKind ("container"/"native_process") -- see ADR-0004/ADR-0015: a
# D-Serv's execution contract is host-independent, and RuntimeKind is the
# OPERATIONAL service-artifact/executor dispatch seam, never canonical
# D-Serv execution semantics.
DNODE_ABI_V0 = "datum-dnode/0"

CANONICAL_DGRAPH_SCHEMA = "datum.dgraph/1"
CANONICAL_DCONTINUUM_SCHEMA = "datum.dcontinuum/1"
CANONICAL_DMAP_SCHEMA = "datum.dmap/1"
PRODUCTION_SCHEMA_IDS = {CANONICAL_DGRAPH_SCHEMA, CANONICAL_DCONTINUUM_SCHEMA, CANONICAL_DMAP_SCHEMA}

EMITTABLE_CODES = {
    "schema_invalid", "missing_field", "revision_invalid",
    "duplicate_service_id", "duplicate_call_id", "dangling_call_source",
    "dangling_call_target",
    # Stage 110 P0 fix (independent review v4, D-Serv host invariance):
    # replaces invalid_runtime_kind/invalid_runtime_capability/
    # duplicate_runtime_capability/incompatible_runtime_capability in this
    # NEW canonical-v1 contract surface only. Stage108/109 RuntimeKind code
    # and its own tests are untouched elsewhere in the repository.
    "invalid_dnode_abi", "invalid_dnode_abi_capability", "duplicate_dnode_abi_capability",
    "incompatible_dnode_abi",
    "duplicate_node_id", "duplicate_link_id", "dangling_link_endpoint",
    "invalid_platform",
    "duplicate_placement", "dgraph_reference_mismatch",
    "dgraph_digest_mismatch", "dcontinuum_reference_mismatch",
    "dcontinuum_digest_mismatch", "application_mismatch", "project_mismatch",
    "unknown_service", "unknown_node",
    "missing_placement",
    # Stage 110 P0 fix (independent review v3, Stage 112 resource readiness):
    "invalid_cpu_requirement", "invalid_memory_requirement",
    "invalid_node_cpu_capacity", "invalid_node_memory_capacity",
    "insufficient_node_cpu", "insufficient_node_memory",
    "aggregate_node_cpu_exceeded", "aggregate_node_memory_exceeded",
    # Stage 110 P1 fix (independent review v3): reference digest shape.
    "invalid_digest_format",
    # Phase 113A (ADR-0023): datum.dforward/1 fixtures happen to live under
    # this same canonical-v1/negative/ directory; these are the exact Rust
    # `canonical_dforward.rs` finding codes this checker's `dforward` branch
    # mirrors -- `deny_unknown_fields` is a pseudo-code standing in for a
    # deserialization-time `#[serde(deny_unknown_fields)]` rejection, exactly
    # like the real Rust fixture-scan test's own special case for it.
    "duplicate_diot_id", "dangling_hop_source", "dangling_hop_target",
    "external_direction_invalid", "external_adjacent_interface_invalid",
    "deny_unknown_fields",
}

_CANONICAL_DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")


def is_canonical_digest_format(value) -> bool:
    """Mirrors `jcs::is_canonical_digest_format` (Rust) exactly: the literal
    `sha256:` prefix followed by exactly 64 lowercase hex characters."""
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
# Per-schema validators (independent Python mirror of the Rust `validate()`   #
# / `validate_against()` methods). Every finding code below matches the      #
# Rust `CanonicalValidationFinding.code` string exactly.                     #
# --------------------------------------------------------------------------- #

def v_canonical_dgraph(obj: dict) -> list[dict]:
    e: list[dict] = []
    if obj.get("schema") != CANONICAL_DGRAPH_SCHEMA:
        e.append(F("schema_invalid", f"schema must be '{CANONICAL_DGRAPH_SCHEMA}', got '{obj.get('schema')}'"))
    if not (isinstance(obj.get("application_id"), str) and obj["application_id"].strip()):
        e.append(F("missing_field", "application_id must be a non-empty string"))
    if not (isinstance(obj.get("dgraph_id"), str) and obj["dgraph_id"].strip()):
        e.append(F("missing_field", "dgraph_id must be a non-empty string"))
    if not isinstance(obj.get("revision"), int) or obj.get("revision", 0) < 1:
        e.append(F("revision_invalid", "revision must be an integer >= 1"))

    seen_service_ids: set[str] = set()
    known_services: set[str] = set()
    for s in obj.get("services") or []:
        sid = s.get("service_id")
        if not (isinstance(sid, str) and sid.strip()):
            e.append(F("missing_field", "service_id must be a non-empty string"))
        elif sid in seen_service_ids:
            e.append(F("duplicate_service_id", f"duplicate service_id '{sid}'"))
        seen_service_ids.add(sid)
        known_services.add(sid)
        abi = (s.get("execution_requirement") or {}).get("dnode_abi")
        if abi != DNODE_ABI_V0:
            e.append(F("invalid_dnode_abi", f"service '{sid}' execution_requirement.dnode_abi '{abi}' is not a recognised D-Node ABI (expected '{DNODE_ABI_V0}')"))
        # Stage 110 P0 fix (independent review v3, Stage 112 resource
        # readiness): mirrors the Rust `resource_requirement` checks exactly.
        resource_requirement = s.get("resource_requirement") or {}
        cpu_cores = resource_requirement.get("cpu_cores")
        if not isinstance(cpu_cores, int) or isinstance(cpu_cores, bool) or cpu_cores < 1:
            e.append(F("invalid_cpu_requirement", f"service '{sid}' resource_requirement.cpu_cores must be >= 1, got {cpu_cores!r}"))
        memory_bytes = resource_requirement.get("memory_bytes")
        if not isinstance(memory_bytes, int) or isinstance(memory_bytes, bool) or memory_bytes < 1:
            e.append(F("invalid_memory_requirement", f"service '{sid}' resource_requirement.memory_bytes must be >= 1, got {memory_bytes!r}"))

    seen_call_ids: set[str] = set()
    for c in obj.get("calls") or []:
        cid = c.get("call_id")
        if not (isinstance(cid, str) and cid.strip()):
            e.append(F("missing_field", "call_id must be a non-empty string"))
        elif cid in seen_call_ids:
            e.append(F("duplicate_call_id", f"duplicate call_id '{cid}'"))
        seen_call_ids.add(cid)
        src, tgt = c.get("source_service_id"), c.get("target_service_id")
        # D-4 (ADR-0001): a cycle -- including a self-call -- is NEVER
        # rejected here. Only a dangling endpoint is.
        if src not in known_services:
            e.append(F("dangling_call_source", f"D-Call '{cid}' source_service_id '{src}' does not name a declared service"))
        if tgt not in known_services:
            e.append(F("dangling_call_target", f"D-Call '{cid}' target_service_id '{tgt}' does not name a declared service"))
    return e


def v_canonical_dcontinuum(obj: dict) -> list[dict]:
    e: list[dict] = []
    if obj.get("schema") != CANONICAL_DCONTINUUM_SCHEMA:
        e.append(F("schema_invalid", f"schema must be '{CANONICAL_DCONTINUUM_SCHEMA}', got '{obj.get('schema')}'"))
    if not (isinstance(obj.get("project_id"), str) and obj["project_id"].strip()):
        e.append(F("missing_field", "project_id must be a non-empty string"))
    if not (isinstance(obj.get("dcontinuum_id"), str) and obj["dcontinuum_id"].strip()):
        e.append(F("missing_field", "dcontinuum_id must be a non-empty string"))
    if not isinstance(obj.get("revision"), int) or obj.get("revision", 0) < 1:
        e.append(F("revision_invalid", "revision must be an integer >= 1"))

    seen_node_ids: set[str] = set()
    known_nodes: set[str] = set()
    for n in obj.get("nodes") or []:
        nid = n.get("node_id")
        if not (isinstance(nid, str) and nid.strip()):
            e.append(F("missing_field", "node_id must be a non-empty string"))
        elif nid in seen_node_ids:
            e.append(F("duplicate_node_id", f"duplicate node_id '{nid}'"))
        seen_node_ids.add(nid)
        known_nodes.add(nid)
        platform = n.get("platform") or {}
        if not (platform.get("os") or "").strip() or not (platform.get("architecture") or "").strip():
            e.append(F("invalid_platform", f"node '{nid}' platform.os/architecture must be non-empty"))
        # Stage 110 P0 fix (independent review v3, Stage 112 resource
        # readiness): mirrors the Rust `capacity` checks exactly.
        capacity = n.get("capacity") or {}
        cpu_cores = capacity.get("cpu_cores")
        if not isinstance(cpu_cores, int) or isinstance(cpu_cores, bool) or cpu_cores < 1:
            e.append(F("invalid_node_cpu_capacity", f"node '{nid}' capacity.cpu_cores must be >= 1, got {cpu_cores!r}"))
        memory_bytes = capacity.get("memory_bytes")
        if not isinstance(memory_bytes, int) or isinstance(memory_bytes, bool) or memory_bytes < 1:
            e.append(F("invalid_node_memory_capacity", f"node '{nid}' capacity.memory_bytes must be >= 1, got {memory_bytes!r}"))
        # Stage 110 P1 fix (independent review v2, re-verified v4):
        # dnode_abi_versions is a documented SET (ADR-0017) -- a duplicate
        # entry is invalid input, mirroring the identical fix in both Rust
        # crates' validate(). An empty set is legal (a restricted node that
        # cannot host any D-Code under the current ABI).
        seen_dnode_abi_versions: set[str] = set()
        for abi in (n.get("capabilities") or {}).get("dnode_abi_versions") or []:
            if abi != DNODE_ABI_V0:
                e.append(F("invalid_dnode_abi_capability", f"node '{nid}' declares unrecognised D-Node ABI capability '{abi}'"))
            elif abi in seen_dnode_abi_versions:
                e.append(F("duplicate_dnode_abi_capability", f"node '{nid}' declares D-Node ABI capability '{abi}' more than once"))
            else:
                seen_dnode_abi_versions.add(abi)

    seen_link_ids: set[str] = set()
    for link in obj.get("links") or []:
        lid = link.get("link_id")
        if not (isinstance(lid, str) and lid.strip()):
            e.append(F("missing_field", "link_id must be a non-empty string"))
        elif lid in seen_link_ids:
            e.append(F("duplicate_link_id", f"duplicate link_id '{lid}'"))
        seen_link_ids.add(lid)
        if link.get("from_node_id") not in known_nodes:
            e.append(F("dangling_link_endpoint", f"link '{lid}' from_node_id '{link.get('from_node_id')}' does not name a declared node"))
        if link.get("to_node_id") not in known_nodes:
            e.append(F("dangling_link_endpoint", f"link '{lid}' to_node_id '{link.get('to_node_id')}' does not name a declared node"))
    return e


def v_canonical_dmap_structural(obj: dict) -> list[dict]:
    """Self-contained checks only -- mirrors Rust `CanonicalDMapV1::validate`."""
    e: list[dict] = []
    if obj.get("schema") != CANONICAL_DMAP_SCHEMA:
        e.append(F("schema_invalid", f"schema must be '{CANONICAL_DMAP_SCHEMA}', got '{obj.get('schema')}'"))
    for field in ("project_id", "application_id", "dmap_id"):
        if not (isinstance(obj.get(field), str) and obj[field].strip()):
            e.append(F("missing_field", f"{field} must be a non-empty string"))
    if not isinstance(obj.get("revision"), int) or obj.get("revision", 0) < 1:
        e.append(F("revision_invalid", "revision must be an integer >= 1"))
    for ref_field in ("dgraph_ref", "dcontinuum_ref"):
        ref = obj.get(ref_field) or {}
        if not (isinstance(ref.get("id"), str) and ref["id"].strip()) or not isinstance(ref.get("revision"), int) or ref.get("revision", 0) < 1:
            e.append(F("missing_field", f"{ref_field}.id/revision must be present and revision >= 1"))
        # Stage 110 P1 fix (independent review v3): reference digest SHAPE,
        # mirrors `jcs::is_canonical_digest_format` used by both Rust crates.
        if not is_canonical_digest_format(ref.get("digest")):
            e.append(F("invalid_digest_format", f"{ref_field}.digest '{ref.get('digest')}' is not the canonical 'sha256:<64 lowercase hex>' form (ADR-0006)"))

    seen_service_ids: set[str] = set()
    for p in obj.get("placements") or []:
        sid = p.get("service_id")
        if not (isinstance(sid, str) and sid.strip()):
            e.append(F("missing_field", "placement.service_id must be a non-empty string"))
        elif sid in seen_service_ids:
            e.append(F("duplicate_placement", f"service '{sid}' is placed more than once; Stage 110/v0.1 requires exactly one placement per D-Serv"))
        seen_service_ids.add(sid)
        if not (isinstance(p.get("node_id"), str) and p["node_id"].strip()):
            e.append(F("missing_field", "placement.node_id must be a non-empty string"))
    return e


def canonicalize_json_text(value) -> str:
    """RFC 8785-equivalent-enough canonical form for THIS checker's own
    digest recomputation: recursively sorted object keys, compact
    separators. Not a general JCS implementation -- only used here to
    independently recompute the digest of the exact fixture content this
    script already trusts to be well-shaped JSON (ints/strings/bools/
    arrays/objects only, matching what the Rust JCS module itself scopes
    to). This mirrors -- does not replace -- the Rust conformance-tested
    JCS module; parity between the two is what the digest fields below
    actually prove.
    """
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def recompute_digest(value) -> str:
    canonical = canonicalize_json_text(value)
    return "sha256:" + hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def digest_of_dgraph(obj: dict) -> str:
    # WHITELIST exactly the Rust `CanonicalDGraphV1` field set. This is
    # deliberate, not merely tidy: Rust's serde deserializer silently DROPS
    # any JSON field with no matching struct field (this project's
    # documented forward-compatible-unknown-fields policy) -- so a fixture's
    # `x_note`/`expected_digest_sha256`/`x_expect_reject` bookkeeping fields
    # are NEVER part of what either Rust crate actually hashes. Hashing the
    # raw loaded dict as-is here would silently include those extra keys and
    # produce a digest that could never match Rust's, defeating the entire
    # point of this cross-check.
    canonical_obj = {
        "schema": obj.get("schema"),
        "application_id": obj.get("application_id"),
        "dgraph_id": obj.get("dgraph_id"),
        "revision": obj.get("revision"),
        "services": sorted(
            (
                {
                    "service_id": s.get("service_id"),
                    "execution_requirement": {"dnode_abi": (s.get("execution_requirement") or {}).get("dnode_abi")},
                    "resource_requirement": {
                        "cpu_cores": (s.get("resource_requirement") or {}).get("cpu_cores"),
                        "memory_bytes": (s.get("resource_requirement") or {}).get("memory_bytes"),
                    },
                }
                for s in (obj.get("services") or [])
            ),
            key=lambda s: s["service_id"] or "",
        ),
        "calls": sorted(
            (
                {
                    "call_id": c.get("call_id"),
                    "source_service_id": c.get("source_service_id"),
                    "target_service_id": c.get("target_service_id"),
                }
                for c in (obj.get("calls") or [])
            ),
            key=lambda c: c["call_id"] or "",
        ),
    }
    return recompute_digest(canonical_obj)


def digest_of_dcontinuum(obj: dict) -> str:
    # Same whitelist discipline as `digest_of_dgraph` -- see its comment.
    canonical_obj = {
        "schema": obj.get("schema"),
        "project_id": obj.get("project_id"),
        "dcontinuum_id": obj.get("dcontinuum_id"),
        "revision": obj.get("revision"),
        "nodes": sorted(
            (
                {
                    "node_id": n.get("node_id"),
                    "platform": {
                        "os": (n.get("platform") or {}).get("os"),
                        "architecture": (n.get("platform") or {}).get("architecture"),
                    },
                    "capacity": {
                        "cpu_cores": (n.get("capacity") or {}).get("cpu_cores"),
                        "memory_bytes": (n.get("capacity") or {}).get("memory_bytes"),
                    },
                    "capabilities": {"dnode_abi_versions": sorted((n.get("capabilities") or {}).get("dnode_abi_versions") or [])},
                }
                for n in (obj.get("nodes") or [])
            ),
            key=lambda n: n["node_id"] or "",
        ),
        "links": sorted(
            (
                {"link_id": link.get("link_id"), "from_node_id": link.get("from_node_id"), "to_node_id": link.get("to_node_id")}
                for link in (obj.get("links") or [])
            ),
            key=lambda link: link["link_id"] or "",
        ),
    }
    return recompute_digest(canonical_obj)


def digest_of_dmap(obj: dict) -> str:
    # Same whitelist discipline as `digest_of_dgraph` -- see its comment.
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


def v_canonical_dmap_cross(dmap: dict, dgraph: dict | None, dcontinuum: dict | None) -> list[dict]:
    e: list[dict] = []
    dg_ref = dmap.get("dgraph_ref") or {}
    if dgraph is None:
        return e  # cross-checks require a resolvable D-Graph; caller supplies None only when unresolved (handled elsewhere for canonical-v1's single-fixture-set model)
    if dg_ref.get("id") != dgraph.get("dgraph_id") or dg_ref.get("revision") != dgraph.get("revision"):
        e.append(F("dgraph_reference_mismatch",
                    f"dgraph_ref {dg_ref} does not match the provided D-Graph "
                    f"{{id: {dgraph.get('dgraph_id')!r}, revision: {dgraph.get('revision')!r}}}"))
    else:
        actual = digest_of_dgraph(dgraph)
        if dg_ref.get("digest") != actual:
            e.append(F("dgraph_digest_mismatch", f"dgraph_ref.digest '{dg_ref.get('digest')}' does not match recomputed digest '{actual}'"))

    dc_ref = dmap.get("dcontinuum_ref") or {}
    if dc_ref.get("id") != dcontinuum.get("dcontinuum_id") or dc_ref.get("revision") != dcontinuum.get("revision"):
        e.append(F("dcontinuum_reference_mismatch",
                    f"dcontinuum_ref {dc_ref} does not match the provided D-Continuum "
                    f"{{id: {dcontinuum.get('dcontinuum_id')!r}, revision: {dcontinuum.get('revision')!r}}}"))
    else:
        actual = digest_of_dcontinuum(dcontinuum)
        if dc_ref.get("digest") != actual:
            e.append(F("dcontinuum_digest_mismatch", f"dcontinuum_ref.digest '{dc_ref.get('digest')}' does not match recomputed digest '{actual}'"))

    if dmap.get("application_id") != dgraph.get("application_id"):
        e.append(F("application_mismatch", f"D-Map application_id '{dmap.get('application_id')}' != D-Graph application_id '{dgraph.get('application_id')}'"))
    if dmap.get("project_id") != dcontinuum.get("project_id"):
        e.append(F("project_mismatch", f"D-Map project_id '{dmap.get('project_id')}' != D-Continuum project_id '{dcontinuum.get('project_id')}'"))

    known_services = {s.get("service_id"): s for s in dgraph.get("services") or []}
    known_nodes = {n.get("node_id"): n for n in dcontinuum.get("nodes") or []}
    placed: set[str] = set()
    for p in dmap.get("placements") or []:
        placed.add(p.get("service_id"))
        service = known_services.get(p.get("service_id"))
        if service is None:
            e.append(F("unknown_service", f"placement names service '{p.get('service_id')}' not present in the referenced D-Graph"))
        node = known_nodes.get(p.get("node_id"))
        if node is None:
            e.append(F("unknown_node", f"placement names node '{p.get('node_id')}' not present in the referenced D-Continuum"))
        if service is not None and node is not None:
            required = (service.get("execution_requirement") or {}).get("dnode_abi")
            supported = (node.get("capabilities") or {}).get("dnode_abi_versions") or []
            if required not in supported:
                e.append(F("incompatible_dnode_abi",
                            f"service '{p.get('service_id')}' requires dnode_abi '{required}', "
                            f"node '{p.get('node_id')}' declares {supported}"))

            # Stage 110 P0 fix (independent review v3, Stage 112 resource
            # readiness): FEASIBILITY validation only, mirrors the Rust
            # individual-fit check exactly.
            req_cpu = (service.get("resource_requirement") or {}).get("cpu_cores") or 0
            node_cpu = (node.get("capacity") or {}).get("cpu_cores") or 0
            if req_cpu > node_cpu:
                e.append(F("insufficient_node_cpu",
                            f"service '{p.get('service_id')}' requires {req_cpu} cpu_cores, "
                            f"node '{p.get('node_id')}' declares capacity.cpu_cores {node_cpu}"))
            req_mem = (service.get("resource_requirement") or {}).get("memory_bytes") or 0
            node_mem = (node.get("capacity") or {}).get("memory_bytes") or 0
            if req_mem > node_mem:
                e.append(F("insufficient_node_memory",
                            f"service '{p.get('service_id')}' requires {req_mem} memory_bytes, "
                            f"node '{p.get('node_id')}' declares capacity.memory_bytes {node_mem}"))

    # Aggregate fit: mirrors the Rust aggregation exactly -- when multiple
    # D-Servs are placed on the SAME node, their combined requirement must
    # not exceed that node's declared capacity, even if each fits alone.
    node_cpu_committed: dict[str, int] = {}
    node_memory_committed: dict[str, int] = {}
    for p in dmap.get("placements") or []:
        node = known_nodes.get(p.get("node_id"))
        service = known_services.get(p.get("service_id"))
        if node is None or service is None:
            continue
        node_id = p.get("node_id")
        req_cpu = (service.get("resource_requirement") or {}).get("cpu_cores") or 0
        req_mem = (service.get("resource_requirement") or {}).get("memory_bytes") or 0
        node_cpu_committed[node_id] = node_cpu_committed.get(node_id, 0) + req_cpu
        node_memory_committed[node_id] = node_memory_committed.get(node_id, 0) + req_mem
    for node_id, total_cpu in node_cpu_committed.items():
        capacity = (known_nodes[node_id].get("capacity") or {}).get("cpu_cores") or 0
        if total_cpu > capacity:
            e.append(F("aggregate_node_cpu_exceeded", f"node '{node_id}' aggregate placed cpu_cores {total_cpu} exceeds declared capacity.cpu_cores {capacity}"))
    for node_id, total_memory in node_memory_committed.items():
        capacity = (known_nodes[node_id].get("capacity") or {}).get("memory_bytes") or 0
        if total_memory > capacity:
            e.append(F("aggregate_node_memory_exceeded", f"node '{node_id}' aggregate placed memory_bytes {total_memory} exceeds declared capacity.memory_bytes {capacity}"))

    for s in dgraph.get("services") or []:
        if s.get("service_id") not in placed:
            e.append(F("missing_placement", f"D-Graph service '{s.get('service_id')}' has no placement in this D-Map"))
    return e


# --------------------------------------------------------------------------- #

def dispatch_findings(path: Path, obj: dict, dgraph_by_key: dict, dcontinuum_by_key: dict) -> list[dict]:
    # Route by FILENAME CONVENTION (`dgraph.*.json` / `dcontinuum.*.json` /
    # `dmap.*.json`), never by the fixture's own (possibly deliberately
    # wrong, for the "wrong schema" negative case) `schema` field -- a
    # schema-field-keyed dispatcher can never route a wrong-schema fixture
    # to the validator that would actually catch it, since routing itself
    # would fail first. The `schema_invalid` check happens INSIDE each
    # validator, exactly like the Rust `validate()` methods.
    kind = path.name.split(".", 1)[0]
    if kind == "dgraph":
        return v_canonical_dgraph(obj)
    if kind == "dcontinuum":
        return v_canonical_dcontinuum(obj)
    if kind == "dmap":
        # Stage 110 P1 fix (independent review v2): this branch is the
        # COMPLETE, fail-closed validation path for a D-Map fixture,
        # mirroring the Rust `CanonicalDMapV1::validate_against` exactly --
        # it aggregates the D-Map's own structural checks, the REFERENCED
        # D-Graph/D-Continuum's own structural checks, AND the cross-
        # reference checks. `v_canonical_dmap_cross` alone (a raw-string
        # comparison) would report nothing for a D-Graph service and
        # D-Continuum node that both declare the SAME invalid runtime
        # string; including `v_canonical_dgraph`/`v_canonical_dcontinuum`'s
        # own findings here closes that gap the same way the Rust fix does.
        findings = v_canonical_dmap_structural(obj)
        # Cross-checked against the one real dgraph/dcontinuum fixture in
        # this v1 set -- deliberately NOT a lookup keyed by the D-Map's own,
        # possibly tampered, dgraph_ref/dcontinuum_ref (that would let a
        # wrong-revision/wrong-digest fixture silently skip cross-checking
        # entirely instead of being caught by it). This mirrors the Rust
        # `validate_against(dgraph, dcontinuum)` signature: the CALLER
        # already knows which real objects to check against; a D-Map is
        # never trusted to name its own reference target for lookup
        # purposes.
        real_dgraph = dgraph_by_key.get(("dgraph:smart-sentinel-lab", 1))
        real_dcontinuum = dcontinuum_by_key.get(("continuum:smart-sentinel-lab", 1))
        if real_dgraph is not None:
            findings += v_canonical_dgraph(real_dgraph)
        if real_dcontinuum is not None:
            findings += v_canonical_dcontinuum(real_dcontinuum)
        if real_dgraph is not None and real_dcontinuum is not None:
            findings += v_canonical_dmap_cross(obj, real_dgraph, real_dcontinuum)
        return findings
    if kind == "dforward":
        findings = []
        # Top-level `#[serde(deny_unknown_fields)]` (Rust) rejects an unknown
        # key at DESERIALIZATION time, before validate() ever runs -- mirrored
        # here as a pseudo-finding so this checker's uniform
        # negative_fixture_verdict() path can still assert it, exactly like
        # the real Rust fixture-scan test's dedicated "deserialized
        # successfully" special case for this one code.
        known_top_level_keys = {
            "schema", "project_id", "dforward_id", "revision",
            "diots", "external_endpoints", "chains", "support_jobs",
        }
        if any(k not in known_top_level_keys for k in obj.keys() if not k.startswith("x_") and k != "expected_digest_sha256"):
            findings.append(F("deny_unknown_fields", "unrecognised top-level field"))
        if obj.get("schema") != "datum.dforward/1": findings.append(F("schema_invalid", "schema must be datum.dforward/1"))
        for key in ("project_id", "dforward_id"):
            if not isinstance(obj.get(key), str) or not obj[key].strip(): findings.append(F("missing_field", f"{key} must be non-empty"))
        if not isinstance(obj.get("revision"), int) or obj["revision"] < 1: findings.append(F("revision_invalid", "revision must be >= 1"))
        ids=[]
        diots_by_id={}
        for diot in obj.get("diots") or []:
            ident=diot.get("diot_id")
            if not isinstance(ident, str) or not ident.strip(): findings.append(F("missing_field", "diot_id must be non-empty"))
            if ident in ids: findings.append(F("duplicate_diot_id", "duplicate diot_id"))
            ids.append(ident)
            diots_by_id[ident]=diot
        endpoints_by_id={e.get("endpoint_id"): e for e in obj.get("external_endpoints") or []}
        declared=set(ids)|set(endpoints_by_id.keys())

        def provides_or_requires(diot: dict, interface: str) -> bool:
            return any(p.get("interface") == interface for p in diot.get("provides") or []) or \
                   any(r.get("interface") == interface for r in diot.get("requires") or [])

        for chain in obj.get("chains") or []:
            for hop in chain.get("hops") or []:
                h_from, h_to, h_iface = hop.get("from"), hop.get("to"), hop.get("interface")
                if h_from not in declared: findings.append(F("dangling_hop_source", "hop source is undeclared"))
                if h_to not in declared: findings.append(F("dangling_hop_target", "hop target is undeclared"))
                if not isinstance(h_iface, str) or not h_iface.strip(): findings.append(F("missing_field", "hop interface must be non-empty"))
                src_ep = endpoints_by_id.get(h_from)
                if src_ep is not None:
                    if src_ep.get("direction") != "input":
                        findings.append(F("external_direction_invalid", "only input endpoints may be hop sources"))
                    target = diots_by_id.get(h_to)
                    if target is not None and not provides_or_requires(target, h_iface):
                        findings.append(F("external_adjacent_interface_invalid", "DIoT adjacent to input lacks the endpoint interface"))
                dst_ep = endpoints_by_id.get(h_to)
                if dst_ep is not None:
                    if dst_ep.get("direction") != "output":
                        findings.append(F("external_direction_invalid", "only output endpoints may be hop targets"))
                    source = diots_by_id.get(h_from)
                    if source is not None and not provides_or_requires(source, h_iface):
                        findings.append(F("external_adjacent_interface_invalid", "DIoT adjacent to output lacks the endpoint interface"))
        return findings
    return [F("unrecognised_schema", f"unrecognised canonical-v1 fixture filename '{path.name}' (expected dgraph./dcontinuum./dmap. prefix)")]


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


def check_fixtures(rep: Report) -> None:
    positives = sorted(p for p in FIXTURE_DIR.glob("*.json"))
    negatives = sorted(p for p in NEG_DIR.glob("*.json")) if NEG_DIR.exists() else []
    if not positives:
        rep.fail("fixtures", f"no positive fixtures found under {FIXTURE_DIR}")
        return

    loaded: dict[Path, dict] = {}
    dgraph_by_key: dict = {}
    dcontinuum_by_key: dict = {}
    for path in positives:
        obj = load_json(path, rep)
        if obj is None:
            continue
        loaded[path] = obj
        schema = obj.get("schema", "")
        if schema == CANONICAL_DGRAPH_SCHEMA:
            dgraph_by_key[(obj.get("dgraph_id"), obj.get("revision"))] = obj
        elif schema == CANONICAL_DCONTINUUM_SCHEMA:
            dcontinuum_by_key[(obj.get("dcontinuum_id"), obj.get("revision"))] = obj

    n_pos_ok = 0
    for path, obj in loaded.items():
        findings = dispatch_findings(path, obj, dgraph_by_key, dcontinuum_by_key)
        if findings:
            for f in findings:
                rep.fail(path.name, f"[{f['code']}] {f['message']}")
        else:
            n_pos_ok += 1
            # Cross-check the recorded expected_digest_sha256 against this
            # script's OWN independent digest recomputation -- a second,
            # Python-side proof that the fixture's recorded digest is
            # reachable by an implementation that shares nothing with
            # either Rust crate's JCS module.
            expected = obj.get("expected_digest_sha256")
            if expected:
                schema = obj.get("schema")
                if schema == CANONICAL_DGRAPH_SCHEMA:
                    actual = digest_of_dgraph(obj)
                elif schema == CANONICAL_DCONTINUUM_SCHEMA:
                    actual = digest_of_dcontinuum(obj)
                elif schema == CANONICAL_DMAP_SCHEMA:
                    actual = digest_of_dmap(obj)
                else:
                    actual = None
                if actual is not None and actual != expected:
                    rep.fail(path.name, f"expected_digest_sha256 '{expected}' does not match this checker's independently recomputed digest '{actual}'")
    rep.note(f"positive fixtures validated clean: {n_pos_ok}/{len(loaded)}")

    n_neg_ok = 0
    for path in negatives:
        obj = load_json(path, rep)
        if obj is None:
            continue
        want = ((obj.get("x_expect_reject") or {}).get("code"))
        findings = dispatch_findings(path, obj, dgraph_by_key, dcontinuum_by_key)
        produced = {f["code"] for f in findings}
        ok, reason = negative_fixture_verdict(want, produced)
        if ok:
            n_neg_ok += 1
        else:
            rep.fail(path.name, reason)
    rep.note(f"negative fixtures asserted their expected code: {n_neg_ok}/{len(negatives)}")


def check_schema_id_uniqueness(rep: Report) -> None:
    """Stage 110 P0 fix (independent review v3) / PASS I + O: prove one
    production schema ID denotes exactly one wire family, across EVERY DATUM
    contract fixture in the repository -- not merely "the vs1 sketch
    directory doesn't claim it" (independent review v2's version of this
    function enumerated exactly two named directories, `canonical-v1/` and
    `vs1/`; a THIRD fixture family added later under
    `scripts/fixtures/datum-contracts/<something-else>/` would have gone
    completely unscanned and could silently reclaim a production schema ID
    without this check ever noticing).

    Walks `scripts/fixtures/datum-contracts/**/*.json` -- the ENTIRE DATUM
    contract fixture tree, whatever directories currently exist under it --
    and for every file that declares a `schema` field equal to one of the
    three production IDs, asserts the file's path is under
    `canonical-v1/` (this module's own positive OR negative fixtures, the
    only family entitled to those three exact strings). Any other fixture
    family -- `vs1/` today, or any future sibling directory -- claiming a
    production ID is a violation, found by its actual parsed `schema` field
    value, never by directory name or filename convention alone.

    `canonical-v1/negative/`'s deliberately WRONG-SCHEMA fixtures remain
    legal: they exist precisely to prove `schema_invalid` fires, so they
    declare something OTHER than a production ID (e.g. `"datum.dgraph/2"`)
    and are simply never counted here.
    """
    violations: list[str] = []
    canonical_v1_schema_counts: dict[str, int] = {}
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
            is_canonical_v1 = path.resolve().is_relative_to(FIXTURE_DIR.resolve())
        except AttributeError:  # pragma: no cover -- Python < 3.9 fallback, not expected here
            is_canonical_v1 = str(path.resolve()).startswith(str(FIXTURE_DIR.resolve()))
        if is_canonical_v1:
            canonical_v1_schema_counts[schema] = canonical_v1_schema_counts.get(schema, 0) + 1
            continue
        try:
            display_path = path.relative_to(REPO_ROOT)
        except ValueError:
            # Not under REPO_ROOT -- e.g. a selftest pointing the scanned
            # root at a temporary directory to prove this function actually
            # detects the regression.
            display_path = path
        violations.append(
            f"{display_path}: declares production schema '{schema}' but does not live under "
            f"'scripts/fixtures/datum-contracts/canonical-v1/' -- this is exactly the schema-ID "
            f"collision Stage 110's P0 fix (ADR-0017) resolved; a fixture family outside "
            f"canonical-v1/ must use the 'datum.x-fixture.<family>-*' spelling instead"
        )

    if violations:
        for v in violations:
            rep.fail("schema-id-uniqueness", v)
    else:
        rep.note(
            f"schema ID uniqueness: production IDs {sorted(canonical_v1_schema_counts)} found only under "
            f"canonical-v1/ ({sum(canonical_v1_schema_counts.values())} fixture(s)) across all DATUM contract "
            f"fixtures; zero occurrences in any other fixture family"
        )
    # Every production ID must actually be exercised at least once, or this
    # check would trivially "pass" by finding nothing.
    missing = PRODUCTION_SCHEMA_IDS - set(canonical_v1_schema_counts)
    if missing:
        rep.fail("schema-id-uniqueness", f"production schema ID(s) {sorted(missing)} not found in any canonical-v1 fixture")


def list_fixtures() -> None:
    print("positive fixtures:")
    for p in sorted(FIXTURE_DIR.glob("*.json")):
        obj = json.loads(p.read_text(encoding="utf-8"))
        print(f"  {p.name:44s} {obj.get('schema')}")
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
    print("[canonical-v1 fixtures]")
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
    print("\nOK: all Stage 110 canonical-v1 contract checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
