#!/usr/bin/env python3
"""Stage 107 consistency checker.

Validates the VS1 contract fixtures under
`scripts/fixtures/datum-contracts/vs1/` and the Stage 107 documentation
corrections. Python 3 standard library only. No runtime code is exercised.

Per-schema validators return **structured findings** `{code, message}`. A positive
fixture passes with zero findings. A negative fixture (under `negative/`) declares
`x_expect_reject.code` and passes only when that exact code is among the produced
finding codes -- an unrelated validation error is not sufficient.

Exit code 0 iff every check passes. See
`docs/STAGE107_DATUM_SPEC_GAP_REPORT_AND_VERTICAL_SLICE_PLAN.md` and
`documentation/adr/` for the rules being enforced. Self-tests live in
`scripts/check_datum_contract_fixtures_selftest.py`.

Usage:
    python scripts/check_datum_contract_fixtures.py          # run all checks
    python scripts/check_datum_contract_fixtures.py --list   # list fixtures
"""

from __future__ import annotations

import json
import hashlib
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
FIXTURE_DIR = REPO_ROOT / "scripts" / "fixtures" / "datum-contracts" / "vs1"
NEG_DIR = FIXTURE_DIR / "negative"
DOC_DIR = REPO_ROOT / "documentation"
ADR_DIR = DOC_DIR / "adr"

PLACEHOLDER_DIGEST = "sha256:" + "0" * 64
SHA256_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
MAPPER_FIXTURE = "execution-projection.cross-crate-mapper.mqtt-broker.json"
# Stage 107B-A (ADR-0013): a second real cross-crate mapper fixture, exercising
# a populated resolved_image_identity (index_digest + one platform manifest).
# Exact-filename-matched, same as MAPPER_FIXTURE -- see the narrow-scoping note
# on the guard loop in check_fixtures() below. Never widen this to a prefix,
# substring, or directory-wide match.
MAPPER_FIXTURE_OCI = "execution-projection.cross-crate-mapper.oci-resolved.json"
MAPPER_FIXTURES = {MAPPER_FIXTURE, MAPPER_FIXTURE_OCI}


def mapper_fixture_digest_valid(obj: dict) -> bool:
    """Check the actual-mapper fixture's recorded JCS bytes and digest carriers.

    Both Rust mapper tests independently assert these exact bytes. This check
    does not replace JCS implementation conformance tests.
    """
    try:
        canonical = obj["expected_canonical_json"]
        digest = "sha256:" + hashlib.sha256(canonical.encode("utf-8")).hexdigest()
        return (
            json.loads(canonical) == obj["expected_projection"]
            and obj["expected_projection_digest_sha256"] == digest
            and obj["agent_operational_artifact_assignment"]["execution_projection_digest"] == digest
            and obj["dserver_service_artifact"]["control_plane"]["execution_projection_digest"] == digest
        )
    except (KeyError, TypeError, ValueError, AttributeError):
        return False

CANONICAL_TERMS = [
    "D-Application", "D-Script", "D-Function", "D-Compile", "D-Graph", "D-Serv",
    "D-Call", "D-Code", "D-Continuum", "D-Map", "D-Controller", "D-Deploy",
    "D-Platform", "D-Engine", "D-Node", "D-Forward", "D-IoT", "D-Monitor",
    "D-Agent",
]
LEGACY_LABELS = [
    "operational deployment document v1",
    "controlled composite graph",
    "observed runtime graph",
]
# Identifier strings of the six canonical v2 contract shapes. Stage 107 provides
# hand-authored *contract fixtures* for these; the typed model and JSON Schemas
# generated from it are Stage 108 work.
V2_CONTRACT_SHAPE_IDS = [
    "datum.dgraph/1", "datum.dcontinuum/1", "datum.dforward/1", "datum.dmap/1",
    "datum.operational-slice/2", "datum.dserv-artifact/1",
]
KNOWN_REJECT_CODES = {
    "dcall_endpoint_not_found", "dcall_payload_schema_mismatch",
    "duplicate_dserv_id", "dserv_missing_dcode",
    "dserv_placement_without_dnode_runtime", "dnode_host_kind_not_supported",
    "dnode_abi_incompatible", "dcall_not_realized", "deployment_dependency_cycle",
    "channel_wildcard_or_malformed", "host_kind_field_in_host_invariant_contract",
    "host_dependent_value_in_execution_contract", "v2_slice_not_required_mode",
    "stage_constraint_violation",
}
HOST_KIND_FORBIDDEN_KEYS = {
    "host_kind", "dnode_runtime_placements", "container_name", "image_identity",
    "configuration_mounts",
}
ADDRESS_LIKE_KEYS = {"broker_address", "address", "url", "endpoint_address", "host"}

# Wording that asserts, as a property that holds, that an artifact change
# immediately prevents not-yet-executed / already-decided mutations. These
# patterns are assertions, never option names or negated statements, so a match
# is always a violation while TD-1 semantics forbid the claim. ADR-0012 itself is
# exempt from this scan (it names and defines the two options).
ASSERTIVE_IMMEDIATE_REVOCATION = [
    re.compile(r"(?i)\b(provide[sd]?|providing|guarantee[sd]?|guaranteeing|"
               r"enforce[sd]?|enforcing|deliver[sd]?|delivering|ensure[sd]?|"
               r"ensuring)\s+immediate\s+revocation"),
    re.compile(r"(?i)immediate\s+revocation\s+of\s+(all\s+)?"
               r"(not[- ]?yet[- ]?executed|already[- ]?decided|pending|in[- ]?flight)"
               r"\s+mutations"),
    re.compile(r"(?i)artifact\s+change\s+(immediately\s+)?prevents?\s+"
               r"(all\s+)?(not[- ]?yet[- ]?executed|already[- ]?decided|pending|"
               r"in[- ]?flight)\s+mutations"),
    re.compile(r"(?i)immediately\s+prevents?\s+(all\s+)?(not[- ]?yet[- ]?executed|"
               r"already[- ]?decided|pending|in[- ]?flight)\s+mutations"),
]


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


def adr_status(text: str) -> str:
    """Return the lower-cased status token from an ADR '- Status: ...' line."""
    m = re.search(r"(?im)^\s*[-*]\s*Status:\s*(.+?)\s*$", text)
    if not m:
        return ""
    return m.group(1).strip().strip("*").strip().lower()


def walk(obj):
    if isinstance(obj, dict):
        yield obj
        for value in obj.values():
            yield from walk(value)
    elif isinstance(obj, list):
        for item in obj:
            yield from walk(item)


def F(code: str, message: str) -> dict:
    """A structured validation finding."""
    return {"code": code, "message": message}


def forbid_keys(obj, forbidden: set[str], errors: list[dict], code: str,
                context: str) -> None:
    for node in walk(obj):
        for key in node:
            if key in forbidden:
                errors.append(F(code, f"{context}: forbidden key '{key}' present"))


# Map a D-Map `validation.<flag> == false` to the finding code it stands for.
DMAP_FLAG_CODE = {
    "all_dserv_placed": "dserv_not_placed",
    "stage_constraints_satisfied": "stage_constraint_violation",
    "dnode_abi_compatible": "dnode_abi_incompatible",
    "dnode_host_kind_supported": "dnode_host_kind_not_supported",
    "dcalls_realizable": "dcall_not_realized",
    "deployment_dependencies_acyclic": "deployment_dependency_cycle",
}

# Every finding code any validator below can emit. Used to check that each
# negative fixture's x_expect_reject.code is actually reachable.
EMITTABLE_CODES = {
    "schema_invalid", "missing_field", "revision_invalid", "dservs_empty",
    "duplicate_dserv_id", "dserv_missing_dcode", "dserv_port_invalid",
    "dcall_endpoint_not_found", "dcall_port_direction_invalid",
    "dcall_payload_schema_mismatch", "host_kind_field_in_host_invariant_contract",
    "stage_order_invalid", "element_stage_undeclared", "element_missing_platform",
    "dnode_descriptor_incomplete", "project_id_invalid",
    "diot_field_missing", "require_scope_invalid", "chain_hop_unresolved",
    "input_unresolved", "dserv_placement_dserv_unknown",
    "dserv_placement_without_dnode_runtime", "dmap_element_unknown",
    "dserv_not_placed", "stage_constraint_violation", "dnode_abi_incompatible",
    "dnode_host_kind_not_supported", "dcall_not_realized",
    "deployment_dependency_cycle", "channel_wildcard_or_malformed",
    "v2_slice_not_required_mode", "workload_class_unknown",
    "dnode_runtime_missing_host_kind", "host_dependent_value_in_execution_contract",
    "execution_contract_realization_incomplete", "dserv_artifact_field_missing",
    "dcode_sha256_missing", "abi_name_invalid", "abi_allows_imports",
    "projection_kind_invalid", "secret_binding_extra_keys",
    "projection_inline_secret", "unrecognised_schema",
}

# --------------------------------------------------------------------------- #
# per-schema validators. Each returns a list of structured findings {code,message}.
# --------------------------------------------------------------------------- #

def v_dgraph(obj: dict) -> list[dict]:
    e: list[dict] = []
    if obj.get("schema") != "datum.dgraph/1":
        e.append(F("schema_invalid", "schema must be datum.dgraph/1"))
    for field in ("application_id", "dgraph_id", "revision"):
        if field not in obj:
            e.append(F("missing_field", f"missing '{field}'"))
    if not isinstance(obj.get("revision"), int) or obj.get("revision", 0) < 1:
        e.append(F("revision_invalid", "revision must be an integer >= 1"))
    forbid_keys(obj, HOST_KIND_FORBIDDEN_KEYS, e,
                "host_kind_field_in_host_invariant_contract", "dgraph")

    dservs = obj.get("dservs") or []
    if not dservs:
        e.append(F("dservs_empty", "dservs must be non-empty"))
    seen: set[str] = set()
    ports: dict[tuple[str, str], dict] = {}
    for d in dservs:
        did = d.get("dserv_id")
        if did in seen:
            e.append(F("duplicate_dserv_id", f"duplicate dserv_id '{did}'"))
        seen.add(did)
        if "dcode" not in d or "sha256" not in (d.get("dcode") or {}):
            e.append(F("dserv_missing_dcode", f"dserv '{did}' missing dcode.sha256"))
        pspec = d.get("ports") or {}
        for direction in ("inputs", "outputs"):
            for p in pspec.get(direction) or []:
                if "port" not in p or "payload_schema" not in p:
                    e.append(F("dserv_port_invalid",
                               f"dserv '{did}' {direction} port missing port/payload_schema"))
                ports[(did, p.get("port"))] = {
                    "direction": "input" if direction == "inputs" else "output",
                    "payload_schema": p.get("payload_schema"),
                }
    for c in obj.get("dcalls") or []:
        frm, to = c.get("from") or {}, c.get("to") or {}
        fkey = (frm.get("dserv_id"), frm.get("port"))
        tkey = (to.get("dserv_id"), to.get("port"))
        if fkey not in ports:
            e.append(F("dcall_endpoint_not_found",
                       f"D-Call '{c.get('dcall_id')}' from {fkey} not found"))
        elif ports[fkey]["direction"] != "output":
            e.append(F("dcall_port_direction_invalid",
                       f"D-Call '{c.get('dcall_id')}' from {fkey} is not an output port"))
        if tkey not in ports:
            e.append(F("dcall_endpoint_not_found",
                       f"D-Call '{c.get('dcall_id')}' to {tkey} not found"))
        elif ports[tkey]["direction"] != "input":
            e.append(F("dcall_port_direction_invalid",
                       f"D-Call '{c.get('dcall_id')}' to {tkey} is not an input port"))
        if fkey in ports and tkey in ports:
            if ports[fkey]["payload_schema"] != ports[tkey]["payload_schema"]:
                e.append(F("dcall_payload_schema_mismatch",
                           f"D-Call '{c.get('dcall_id')}' payload_schema mismatch"))
    return e


def v_dcontinuum(obj: dict) -> list[dict]:
    e: list[dict] = []
    if obj.get("schema") != "datum.dcontinuum/1":
        e.append(F("schema_invalid", "schema must be datum.dcontinuum/1"))
    # continuum_id and revision are canonical; project_id is an implementation
    # scoping field (ADR-0001) -- accepted, but must be a non-empty string if given.
    for field in ("continuum_id", "revision"):
        if field not in obj:
            e.append(F("missing_field", f"missing '{field}'"))
    if "project_id" in obj and not (isinstance(obj["project_id"], str) and obj["project_id"].strip()):
        e.append(F("project_id_invalid", "project_id, when present, must be a non-empty string"))
    stage_ids = {s.get("stage_id") for s in obj.get("stages") or []}
    for s in obj.get("stages") or []:
        if not isinstance(s.get("order"), int):
            e.append(F("stage_order_invalid", f"stage '{s.get('stage_id')}' order must be integer"))
    for el in obj.get("elements") or []:
        if el.get("stage_id") not in stage_ids:
            e.append(F("element_stage_undeclared",
                       f"element '{el.get('element_id')}' stage '{el.get('stage_id')}' not declared"))
        if "platform" not in el:
            e.append(F("element_missing_platform",
                       f"element '{el.get('element_id')}' missing platform"))
        for dn in el.get("declared_dnodes") or []:
            for field in ("dnode_id", "abi_versions", "supported_host_kinds", "interface"):
                if field not in dn:
                    e.append(F("dnode_descriptor_incomplete",
                               f"dnode '{dn.get('dnode_id')}' missing '{field}'"))
    return e


def v_dforward(obj: dict) -> list[dict]:
    e: list[dict] = []
    if obj.get("schema") != "datum.dforward/1":
        e.append(F("schema_invalid", "schema must be datum.dforward/1"))
    for field in ("dforward_id", "revision"):
        if field not in obj:
            e.append(F("missing_field", f"missing '{field}'"))
    diot_ids: set[str] = set()
    for d in obj.get("diots") or []:
        diot_ids.add(d.get("diot_id"))
        for field in ("diot_id", "diot_kind", "artifact", "provides", "requires"):
            if field not in d:
                e.append(F("diot_field_missing", f"diot '{d.get('diot_id')}' missing '{field}'"))
        for r in d.get("requires") or []:
            if r.get("scope") not in ("same_element", "any_element"):
                e.append(F("require_scope_invalid",
                           f"diot '{d.get('diot_id')}' require scope must be same_element|any_element"))
    external = {ep.get("endpoint_id") for ep in obj.get("external_endpoints") or []}
    resolvable = diot_ids | external
    for chain in obj.get("chains") or []:
        for hop in chain.get("hops") or []:
            for role in ("from", "to"):
                ident = hop.get(role)
                if ident not in resolvable:
                    e.append(F("chain_hop_unresolved",
                               f"chain '{chain.get('chain_id')}' hop {role}='{ident}' does not "
                               f"resolve to a declared diot_id or an external_endpoints entry"))
    return e


def v_dmap(obj: dict, idx: dict) -> list[dict]:
    e: list[dict] = []
    if obj.get("schema") != "datum.dmap/1":
        e.append(F("schema_invalid", "schema must be datum.dmap/1"))
    for field in ("project_id", "dmap_id", "revision", "inputs", "decision",
                  "dserv_placements", "dnode_runtime_placements",
                  "dcall_realizations", "external_port_bindings", "validation"):
        if field not in obj:
            e.append(F("missing_field", f"missing '{field}'"))

    val = obj.get("validation") or {}
    for k, v in val.items():
        if isinstance(v, bool) and v is False:
            e.append(F(DMAP_FLAG_CODE.get(k, "missing_field"),
                       f"validation flag '{k}' is false"))

    inputs = obj.get("inputs") or {}
    dg_ref = inputs.get("dgraph") or {}
    dgraph = idx["dgraph"].get((dg_ref.get("dgraph_id"), dg_ref.get("revision")))
    dc_ref = inputs.get("dcontinuum") or {}
    dcont = idx["dcontinuum"].get((dc_ref.get("continuum_id"), dc_ref.get("revision")))
    df_ref = inputs.get("dforward") or {}
    if not idx["dforward"].get((df_ref.get("dforward_id"), df_ref.get("revision"))):
        e.append(F("input_unresolved", f"inputs.dforward {df_ref} does not resolve to a loaded fixture"))
    if dgraph is None:
        e.append(F("input_unresolved", f"inputs.dgraph {dg_ref} does not resolve to a loaded fixture"))
    if dcont is None:
        e.append(F("input_unresolved", f"inputs.dcontinuum {dc_ref} does not resolve to a loaded fixture"))

    runtime_dnodes = {p.get("dnode_id") for p in obj.get("dnode_runtime_placements") or []}
    element_stage: dict[str, str] = {}
    element_dnode_abis: dict[str, set[str]] = {}
    element_dnode_hostkinds: dict[str, set[str]] = {}
    simulated: set[str] = set()
    if dcont:
        for el in dcont.get("elements") or []:
            element_stage[el.get("element_id")] = el.get("stage_id")
            for dn in el.get("declared_dnodes") or []:
                element_dnode_abis.setdefault(el.get("element_id"), set()).update(dn.get("abi_versions") or [])
                element_dnode_hostkinds.setdefault(el.get("element_id"), set()).update(dn.get("supported_host_kinds") or [])
        for el in dcont.get("simulated_elements") or []:
            simulated.add(el.get("element_id"))

    dgraph_dservs: dict[str, dict] = {}
    dcall_ids: set[str] = set()
    if dgraph:
        for d in dgraph.get("dservs") or []:
            dgraph_dservs[d.get("dserv_id")] = d
        dcall_ids = {c.get("dcall_id") for c in dgraph.get("dcalls") or []}

    for p in obj.get("dserv_placements") or []:
        did = p.get("dserv_id")
        if dgraph and did not in dgraph_dservs:
            e.append(F("dserv_placement_dserv_unknown",
                       f"placement '{p.get('placement_id')}' dserv '{did}' not in D-Graph"))
        if p.get("dnode_id") not in runtime_dnodes:
            e.append(F("dserv_placement_without_dnode_runtime",
                       f"placement '{p.get('placement_id')}' dnode '{p.get('dnode_id')}' "
                       f"has no dnode_runtime_placement"))
        el = p.get("element_id")
        if dcont and el not in element_stage and el not in simulated:
            e.append(F("dmap_element_unknown",
                       f"placement '{p.get('placement_id')}' element '{el}' not in D-Continuum"))
        if dgraph and did in dgraph_dservs and dcont:
            allowed = set((dgraph_dservs[did].get("constraints") or {}).get("allowed_stages") or [])
            stg = element_stage.get(el)
            if allowed and (stg is None or stg not in allowed):
                e.append(F("stage_constraint_violation",
                           f"placement '{p.get('placement_id')}' element '{el}' stage '{stg}' "
                           f"not in {sorted(allowed)}"))
        requested = p.get("x_requested_abi")
        if dcont and el in element_dnode_abis:
            abis = element_dnode_abis[el]
            need = requested or "datum-dnode/0"
            if need not in abis:
                e.append(F("dnode_abi_incompatible",
                           f"placement '{p.get('placement_id')}' abi '{need}' not in dnode "
                           f"abi_versions {sorted(abis)}"))

    for rp in obj.get("dnode_runtime_placements") or []:
        el = rp.get("element_id")
        kind = (rp.get("host") or {}).get("kind")
        if dcont and el in element_dnode_hostkinds and kind not in element_dnode_hostkinds[el]:
            e.append(F("dnode_host_kind_not_supported",
                       f"dnode_runtime_placement '{rp.get('placement_id')}' host kind '{kind}' "
                       f"not in {sorted(element_dnode_hostkinds[el])}"))

    realized = {r.get("dcall_id") for r in obj.get("dcall_realizations") or []}
    for cid in dcall_ids:
        if cid not in realized:
            e.append(F("dcall_not_realized",
                       f"D-Call '{cid}' from D-Graph has no dcall_realizations entry"))

    for r in obj.get("dcall_realizations") or []:
        ch = r.get("channel") or ""
        if "+" in ch or "#" in ch:
            e.append(F("channel_wildcard_or_malformed",
                       f"dcall_realizations channel '{ch}' contains an MQTT wildcard"))
    for b in obj.get("external_port_bindings") or []:
        ch = b.get("channel") or ""
        if "+" in ch or "#" in ch:
            e.append(F("channel_wildcard_or_malformed",
                       f"external_port_bindings channel '{ch}' contains an MQTT wildcard"))

    deps = obj.get("x_derived_deployment_dependencies")
    if deps and _has_cycle(deps):
        e.append(F("deployment_dependency_cycle",
                   "derived deployment-dependency graph has a cycle"))
    return e


def _has_cycle(edges: list[dict]) -> bool:
    graph: dict[str, list[str]] = {}
    for edge in edges:
        graph.setdefault(edge.get("from"), []).append(edge.get("to"))
    colour: dict[str, int] = {}

    def dfs(node: str) -> bool:
        colour[node] = 1
        for nxt in graph.get(node, []):
            if colour.get(nxt) == 1:
                return True
            if colour.get(nxt) is None and dfs(nxt):
                return True
        colour[node] = 2
        return False

    return any(colour.get(n) is None and dfs(n) for n in list(graph))


def v_operational_slice(obj: dict) -> list[dict]:
    e: list[dict] = []
    if obj.get("schema") != "datum.operational-slice/2":
        e.append(F("schema_invalid", "schema must be datum.operational-slice/2"))
    if obj.get("execution_pinning_mode") != "required":
        e.append(F("v2_slice_not_required_mode",
                   "v2 operational slice execution_pinning_mode must be 'required'"))
    auth = obj.get("authorization") or {}
    if auth.get("execution_pinning_mode") not in (None, "required"):
        e.append(F("v2_slice_not_required_mode",
                   "v2 slice authorization execution_pinning_mode must be 'required'"))
    classes = {"diot", "dnode_runtime", "dserv_instance"}
    for w in obj.get("workloads") or []:
        wc = w.get("workload_class")
        if wc not in classes:
            e.append(F("workload_class_unknown",
                       f"workload '{w.get('workload_id')}' unknown class '{wc}'"))
        if wc == "dnode_runtime" and "kind" not in (w.get("host") or {}):
            e.append(F("dnode_runtime_missing_host_kind",
                       f"dnode_runtime workload '{w.get('workload_id')}' missing host.kind"))
        if wc == "dserv_instance":
            ec = w.get("execution_contract") or {}
            for f in v_execution_contract(ec):
                e.append(F(f["code"], f"{w.get('workload_id')}: {f['message']}"))
    return e


def v_execution_contract(obj: dict) -> list[dict]:
    e: list[dict] = []
    if obj.get("schema") != "datum.dserv-execution-contract/1":
        e.append(F("schema_invalid", "schema must be datum.dserv-execution-contract/1"))
    forbid_keys(obj, {"host_kind", "host", "container", "image_identity"}, e,
                "host_kind_field_in_host_invariant_contract", "execution_contract")
    for p in obj.get("ports") or []:
        r = p.get("realization") or {}
        if "diot_placement_id" not in r or "interface" not in r:
            e.append(F("execution_contract_realization_incomplete",
                       f"port '{p.get('port')}' realization must carry diot_placement_id + interface"))
        for k in r:
            if k in ADDRESS_LIKE_KEYS:
                e.append(F("host_dependent_value_in_execution_contract",
                           f"port '{p.get('port')}' realization has address-like key '{k}'"))
    return e


def v_dserv_artifact(obj: dict) -> list[dict]:
    e: list[dict] = []
    if obj.get("schema") != "datum.dserv-artifact/1":
        e.append(F("schema_invalid", "schema must be datum.dserv-artifact/1"))
    for field in ("artifact_id", "version", "dserv_id", "dcode", "abi", "ports"):
        if field not in obj:
            e.append(F("dserv_artifact_field_missing", f"missing '{field}'"))
    if "sha256" not in (obj.get("dcode") or {}):
        e.append(F("dcode_sha256_missing", "dcode.sha256 is mandatory"))
    abi = obj.get("abi") or {}
    if abi.get("name") != "datum-dnode":
        e.append(F("abi_name_invalid", "abi.name must be 'datum-dnode'"))
    if abi.get("allowed_imports") not in ([], None):
        e.append(F("abi_allows_imports", "abi.allowed_imports must be [] for datum-dnode/0"))
    forbid_keys(obj, {"host_kind", "host", "runtime", "container",
                      "image_identity", "configuration_mounts"}, e,
                "host_kind_field_in_host_invariant_contract", "dserv-artifact")
    return e


def v_projection(obj: dict) -> list[dict]:
    e: list[dict] = []
    if obj.get("projection") != "datum.artifact-execution-projection/1":
        e.append(F("projection_kind_invalid",
                   "projection must be datum.artifact-execution-projection/1"))
    for sb in obj.get("secret_bindings") or []:
        extra = set(sb) - {"secret_ref", "target_environment_variable", "version"}
        if extra:
            e.append(F("secret_binding_extra_keys",
                       f"secret_bindings entry has unexpected keys {sorted(extra)} "
                       f"(values must stay local)"))
    for node in walk(obj):
        for k, v in node.items():
            if k in {"secret_value", "password", "token", "api_key"} and isinstance(v, str) and v:
                e.append(F("projection_inline_secret",
                           f"projection contains an inline secret-looking value under '{k}'"))
    return e


# --------------------------------------------------------------------------- #

def check_fixtures(rep: Report) -> None:
    positives = sorted(p for p in FIXTURE_DIR.glob("*.json"))
    negatives = sorted(p for p in NEG_DIR.glob("*.json"))
    if not positives:
        rep.fail("fixtures", f"no positive fixtures found under {FIXTURE_DIR}")
        return

    idx = {"dgraph": {}, "dcontinuum": {}, "dforward": {}, "dserv_artifact": {}}
    loaded: dict[Path, dict] = {}
    for path in positives:
        obj = load_json(path, rep)
        if obj is None:
            continue
        loaded[path] = obj
        schema = obj.get("schema", "")
        if schema == "datum.dgraph/1":
            idx["dgraph"][(obj.get("dgraph_id"), obj.get("revision"))] = obj
        elif schema == "datum.dcontinuum/1":
            idx["dcontinuum"][(obj.get("continuum_id"), obj.get("revision"))] = obj
        elif schema == "datum.dforward/1":
            idx["dforward"][(obj.get("dforward_id"), obj.get("revision"))] = obj
        elif schema == "datum.dserv-artifact/1":
            idx["dserv_artifact"][obj.get("artifact_id")] = obj

    # placeholder-digest guard (positives only). Exact-filename-scoped
    # exemption ONLY for the two known real cross-crate mapper fixtures
    # (MAPPER_FIXTURES): `mapper_exempt` is reset to False for every file in
    # this loop, and only ever becomes True for a file whose name is exactly
    # one of MAPPER_FIXTURES AND whose recorded JCS bytes/digest carriers
    # independently self-verify via mapper_fixture_digest_valid(). An
    # ordinary Stage 107 v2 fixture -- any name outside MAPPER_FIXTURES --
    # can never satisfy this, so a real-looking digest anywhere in it still
    # fails below. Do not widen this to a prefix/substring/glob match.
    for path, obj in loaded.items():
        mapper_exempt = False
        if path.name in MAPPER_FIXTURES:
            if mapper_fixture_digest_valid(obj):
                mapper_exempt = True
            else:
                rep.fail(path.name, "Stage 107 mapper fixture canonical bytes/digest carriers disagree")
        for node in walk(obj):
            for v in node.values():
                if isinstance(v, str) and SHA256_RE.match(v) and v != PLACEHOLDER_DIGEST and not mapper_exempt:
                    rep.fail(path.name, f"unexpected real-looking digest '{v}' in a Stage 107 fixture")

    unreachable = KNOWN_REJECT_CODES - EMITTABLE_CODES
    if unreachable:
        rep.fail("checker", f"KNOWN_REJECT_CODES not emittable by any validator: {sorted(unreachable)}")

    n_pos_ok = 0
    for path, obj in loaded.items():
        if path.name == "README.md":
            continue
        findings = dispatch_findings(obj, idx)
        if findings:
            for f in findings:
                rep.fail(path.name, f"[{f['code']}] {f['message']}")
        else:
            n_pos_ok += 1
    rep.note(f"positive fixtures validated clean: {n_pos_ok}/{len(loaded)}")

    # negative fixtures: each PASSES only if its x_expect_reject.code is among the
    # produced finding codes. Extra unrelated findings are allowed.
    n_neg_ok = 0
    for path in negatives:
        obj = load_json(path, rep)
        if obj is None:
            continue
        want = ((obj.get("x_expect_reject") or {}).get("code"))
        ok, reason = negative_fixture_verdict(want, finding_codes(dispatch_findings(obj, idx)))
        if ok:
            n_neg_ok += 1
        else:
            rep.fail(path.name, reason)
    rep.note(f"negative fixtures asserted their expected code: {n_neg_ok}/{len(negatives)}")

    check_host_invariance(rep)


def dispatch_findings(obj: dict, idx: dict) -> list[dict]:
    schema = obj.get("schema") or ""
    projection = obj.get("projection") or ""
    if schema == "datum.dgraph/1":
        return v_dgraph(obj)
    if schema == "datum.dcontinuum/1":
        return v_dcontinuum(obj)
    if schema == "datum.dforward/1":
        return v_dforward(obj)
    if schema == "datum.dmap/1":
        return v_dmap(obj, idx)
    if schema == "datum.operational-slice/2":
        return v_operational_slice(obj)
    if schema == "datum.dserv-execution-contract/1":
        return v_execution_contract(obj)
    if schema == "datum.dserv-artifact/1":
        return v_dserv_artifact(obj)
    if projection == "datum.artifact-execution-projection/1":
        return v_projection(obj)
    if schema.startswith("datum.x-fixture."):
        return []  # explanatory fixtures: structural JSON only
    return [F("unrecognised_schema", f"unrecognised schema/projection '{schema or projection}'")]


def finding_codes(findings: list[dict]) -> set[str]:
    return {f["code"] for f in findings}


def negative_fixture_verdict(want_code: str | None, produced: set[str]) -> tuple[bool, str]:
    """A negative fixture passes only when its declared x_expect_reject.code is in
    the produced finding codes. An unrelated validation error is NOT enough."""
    if not want_code:
        return False, "negative fixture is missing x_expect_reject.code"
    if want_code not in KNOWN_REJECT_CODES:
        return False, f"x_expect_reject.code '{want_code}' is not a known rejection code"
    if want_code not in EMITTABLE_CODES:
        return False, f"x_expect_reject.code '{want_code}' is not emittable by any validator"
    if want_code in produced:
        return True, ""
    if produced:
        return False, (f"produced findings {sorted(produced)} but not the expected "
                       f"'{want_code}' (an unrelated error does not prove the intended failure)")
    return False, f"negative fixture unexpectedly passed (expected code '{want_code}')"


def check_host_invariance(rep: Report) -> None:
    a = FIXTURE_DIR / "operational-slice.fog-01.container.json"
    b = FIXTURE_DIR / "operational-slice.fog-01.native.json"
    if not a.exists() or not b.exists():
        rep.fail("host-invariance", "container/native slice fixtures missing")
        return
    oa, ob = json.loads(a.read_text(encoding="utf-8")), json.loads(b.read_text(encoding="utf-8"))

    def contracts(obj: dict) -> dict[str, dict]:
        out: dict[str, dict] = {}
        for w in obj.get("workloads") or []:
            if w.get("workload_class") == "dserv_instance":
                ec = w.get("execution_contract") or {}
                out[ec.get("instance_id")] = ec
        return out

    def dnode_host_kind(obj: dict) -> str | None:
        for w in obj.get("workloads") or []:
            if w.get("workload_class") == "dnode_runtime":
                return (w.get("host") or {}).get("kind")
        return None

    ca, cb = contracts(oa), contracts(ob)
    if set(ca) != set(cb):
        rep.fail("host-invariance", f"instance sets differ: {sorted(ca)} vs {sorted(cb)}")
    for iid in set(ca) & set(cb):
        if json.dumps(ca[iid], sort_keys=True) != json.dumps(cb[iid], sort_keys=True):
            rep.fail("host-invariance",
                     f"DServExecutionContractV1 for '{iid}' differs between container and native host")
    ka, kb = dnode_host_kind(oa), dnode_host_kind(ob)
    if ka == kb:
        rep.fail("host-invariance", f"dnode_runtime host kind is identical ({ka}); the pair must differ")
    elif not (ka == "container" and kb == "native_process"):
        rep.fail("host-invariance", f"unexpected host-kind pair: {ka} / {kb}")
    else:
        rep.note(f"host invariance: execution contracts identical; host kinds differ ({ka} vs {kb})")


def check_documentation(rep: Report) -> None:
    glossary = DOC_DIR / "reference" / "glossary.md"
    if not glossary.exists():
        rep.fail("documentation", "documentation/reference/glossary.md missing")
    else:
        text = glossary.read_text(encoding="utf-8")
        for term in CANONICAL_TERMS + ["Wave"]:
            if f"**{term}**" not in text:
                rep.fail("glossary", f"canonical term '{term}' not defined (expected a **{term}** entry)")
        for label in LEGACY_LABELS:
            if label not in text:
                rep.fail("glossary", f"legacy label '{label}' not present")

    readme = ADR_DIR / "README.md"
    if not readme.exists():
        rep.fail("documentation", "documentation/adr/README.md missing")
    else:
        rtext = readme.read_text(encoding="utf-8")
        for adr in sorted(ADR_DIR.glob("ADR-*.md")):
            num = adr.name.split("-")[1]
            if adr.name not in rtext and f"ADR-{num}" not in rtext:
                rep.fail("adr-index", f"{adr.name} is not listed in documentation/adr/README.md")

    check_td1(rep)

    model = DOC_DIR / "reference" / "datum-canonical-model.md"
    if not model.exists():
        rep.fail("documentation", "documentation/reference/datum-canonical-model.md missing")
    else:
        mtext = model.read_text(encoding="utf-8")
        for sid in V2_CONTRACT_SHAPE_IDS:
            if sid not in mtext:
                rep.fail("canonical-model", f"v2 contract shape id '{sid}' not documented")

    if not (DOC_DIR / "reference" / "vs1-contract-fixtures.md").exists():
        rep.fail("documentation", "documentation/reference/vs1-contract-fixtures.md missing")


def immediate_revocation_claim(text: str) -> str | None:
    """Return the first assertive 'immediate revocation' phrase in `text`, else None."""
    for rx in ASSERTIVE_IMMEDIATE_REVOCATION:
        m = rx.search(text)
        if m:
            return " ".join(m.group(0).split())
    return None


def evaluate_td1(adr_text: str, scanned: dict[str, str]) -> list[tuple[str, str]]:
    """Pure TD-1 (ADR-0012) lifecycle evaluation.

    States and the rule for immediate-revocation claims in contracts, fixtures
    and Stage 107 documentation:

    1. Deferred / Open
       - ADR must NOT record a '- TD-1 resolution:' option.
       - ADR must carry the 'immediate revocation must not be claimed' constraint
         and a 'decided before Stage 107A' marker.
       - Immediate-revocation claims are FORBIDDEN.

    2. Accepted -- Option A (snapshot / lease)
       - ADR records '- TD-1 resolution: Option A' and its test obligations.
       - Immediate-revocation claims REMAIN FORBIDDEN. Documentation may only
         claim the semantics actually provided (immutable authorized snapshot /
         bounded lease / bounded staleness).

    3. Accepted -- Option B (immediate revocation)
       - ADR records '- TD-1 resolution: Option B' and its test obligations.
       - Immediate-revocation claims are permitted ONLY when the ADR also records
         '- TD-1 Option B evidence: satisfied' (or complete / completed /
         recorded). Selecting Option B alone does not enable the claim.

    `adr_text`  : contents of ADR-0012.
    `scanned`   : {label: text} of every contract/fixture/Stage-107 doc to scan
                  for an assertive immediate-revocation claim.
    Returns a list of (where, message) failures.
    """
    fails: list[tuple[str, str]] = []
    status = adr_status(adr_text)
    deferred = status in {"deferred", "open", "deferred / open", "deferred/open"}
    accepted = status.startswith("accepted")
    resolution = re.search(r"(?im)^\s*[-*]\s*TD-1 resolution:\s*(Option [AB])\b", adr_text)
    option = resolution.group(1) if resolution else None
    has_obligations = "test obligations" in adr_text.lower()
    optionb_evidence = bool(re.search(
        r"(?im)^\s*[-*]\s*TD-1 Option B evidence:\s*"
        r"(satisfied|complete|completed|recorded)\b", adr_text))

    claims_permitted = False

    if deferred:
        if resolution:
            fails.append(("adr-0012",
                          "Deferred, but records a '- TD-1 resolution:' option; "
                          "selecting a TD-1 option is Stage 107A work"))
        if not re.search(
            r"(?is)(immediate revocation.{0,160}(must not be claimed|not be claimed|no document))"
            r"|((must not be claimed|no document).{0,160}immediate revocation)", adr_text):
            fails.append(("adr-0012",
                          "Deferred ADR must state that immediate revocation must not be "
                          "claimed while TD-1 is unresolved"))
        if not re.search(r"(?i)before\s+(stage\s+)?107a\s+(implementation|design)", adr_text):
            fails.append(("adr-0012",
                          "Deferred ADR must record that TD-1 is decided before Stage 107A"))
    elif accepted:
        if not resolution:
            fails.append(("adr-0012",
                          "Accepted, but records no '- TD-1 resolution: Option A|B'"))
        if not has_obligations:
            fails.append(("adr-0012",
                          "Accepted, but records no test obligations for the selected semantics"))
        if option == "Option A":
            claims_permitted = False
        elif option == "Option B":
            claims_permitted = optionb_evidence
            if not optionb_evidence:
                # Not itself a failure: Option B may be selected while its
                # implementation/test evidence is still pending. The claim scan
                # below stays active until the evidence marker is present.
                pass
        # option is None -> already flagged; keep claims forbidden.
    else:
        fails.append(("adr-0012",
                      f"status '{status or '(none)'}' not recognised "
                      f"(expected Deferred/Open or Accepted)"))

    if not claims_permitted:
        for label, text in scanned.items():
            hit = immediate_revocation_claim(text)
            if hit:
                fails.append((label,
                              f"asserts immediate revocation ('{hit}') which the current "
                              f"TD-1 state does not permit"))
    return fails


def _td1_scan_targets() -> dict[str, str]:
    """Every contract fixture and Stage 107 doc to scan for immediate-revocation
    claims. ADR-0012 itself is excluded (it defines the options)."""
    targets: dict[str, str] = {}
    for path in list(FIXTURE_DIR.glob("*.json")) + list(NEG_DIR.glob("*.json")):
        try:
            targets[path.name] = path.read_text(encoding="utf-8")
        except OSError:
            pass
    for path in sorted(ADR_DIR.glob("*.md")):
        if path.name == "ADR-0012-toctou-authorization-validity.md":
            continue
        targets[path.name] = path.read_text(encoding="utf-8")
    for name in ("datum-canonical-model.md", "vs1-contract-fixtures.md"):
        path = DOC_DIR / "reference" / name
        if path.exists():
            targets[name] = path.read_text(encoding="utf-8")
    return targets


def check_td1(rep: Report) -> None:
    toctou = ADR_DIR / "ADR-0012-toctou-authorization-validity.md"
    if not toctou.exists():
        rep.fail("documentation", "ADR-0012 (TOCTOU / TD-1) missing")
        return
    for where, msg in evaluate_td1(toctou.read_text(encoding="utf-8"), _td1_scan_targets()):
        rep.fail(where, msg)


def list_fixtures() -> None:
    print("positive fixtures:")
    for p in sorted(FIXTURE_DIR.glob("*.json")):
        obj = json.loads(p.read_text(encoding="utf-8"))
        print(f"  {p.name:52s} {obj.get('schema') or obj.get('projection')}")
    print("negative fixtures:")
    for p in sorted(NEG_DIR.glob("*.json")):
        obj = json.loads(p.read_text(encoding="utf-8"))
        code = (obj.get("x_expect_reject") or {}).get("code")
        print(f"  {p.name:52s} expect_reject={code}")


def main(argv: list[str]) -> int:
    if "--list" in argv:
        list_fixtures()
        return 0

    rep = Report()
    print("[fixtures]")
    check_fixtures(rep)
    print("[documentation]")
    check_documentation(rep)

    for line in rep.notes:
        print(f"  - {line}")
    if rep.failures:
        print(f"\nFAIL: {len(rep.failures)} problem(s)")
        for f in rep.failures:
            print(f"  ! {f}")
        return 1
    print("\nOK: all Stage 107 consistency checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
