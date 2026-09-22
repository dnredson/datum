"""Phase 113A fixture/schema guardrails.

This checker complements (never replaces) the real Rust contract tests in
`canonical_dforward.rs`/`canonical_dmap_v2.rs`/`ddeploy.rs`, which are the
authoritative proof that a negative fixture's claimed rejection is actually
reproduced by the real validator/deserializer. This Python checker instead
proves cheaper, structural invariants across the whole fixture tree:

  - every production-schema fixture (positive OR negative) belongs to its
    one legitimate schema-family directory, never a foreign family;
  - every positive fixture carries the required top-level fields for its
    schema;
  - every negative fixture declares an `x_expect_reject` block naming which
    failure it exercises, so a negative fixture can never silently exist
    without an explicit, checkable claim;
  - a small set of cheap, purely-structural negative shapes (wrong schema,
    an unrecognised/rogue top-level field) are independently verified here
    too, as a second, differently-implemented check on top of the Rust
    test's semantic one -- this checker does NOT re-implement the full
    Rust validator (duplicate placements, digest mismatches, interface
    compatibility, etc.), which remains exclusively the Rust tests' job.
"""
from pathlib import Path
import json

ROOT = Path(__file__).resolve().parent / "fixtures" / "datum-contracts"
PRODUCTION = {"datum.dforward/1", "datum.dmap/2", "datum.ddeploy-proposal/2", "datum.ddeploy-acceptance/2"}
ALLOWED = {"datum.dforward/1": "canonical-v1", "datum.dmap/2": "dmap-v2", "datum.ddeploy-proposal/2": "ddeploy-v2", "datum.ddeploy-acceptance/2": "ddeploy-v2"}
REQUIRED_FIELDS = {
    "datum.dforward/1": ("project_id", "dforward_id", "revision", "diots", "external_endpoints", "chains", "support_jobs"),
    "datum.dmap/2": ("project_id", "application_id", "dmap_id", "revision", "dgraph_ref", "dcontinuum_ref", "dforward_ref", "placements", "diot_placements", "dcall_realizations", "external_port_bindings"),
    "datum.ddeploy-proposal/2": ("proposal_id", "project_id", "application_id", "source", "created_at_utc", "dgraph_ref", "dcontinuum_ref", "dforward_ref", "placements", "diot_placements", "dcall_realizations", "external_port_bindings", "diot_artifact_authority"),
    "datum.ddeploy-acceptance/2": ("acceptance_id", "proposal_id", "proposal_digest", "project_id", "application_id", "accepted_at_utc", "accepted_by", "materialized_dmap_ref"),
}
# Every recognised top-level key across ALL four production schemas, plus the
# fixture-bookkeeping keys every fixture (positive or negative) is allowed to
# carry. Used only for the cheap structural "does this fixture look like it
# has a rogue key" check below -- not a full per-schema field-set validator
# (that remains the Rust `#[serde(deny_unknown_fields)]` structs' job).
KNOWN_KEYS = {"schema"} | {k for fields in REQUIRED_FIELDS.values() for k in fields} | {
    "x_note", "expected_digest_sha256", "x_expect_reject",
}


def schema_locations(root=ROOT):
    found = {}
    for p in root.rglob("*.json"):
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
        except Exception:
            continue
        schema = data.get("schema") if isinstance(data, dict) else None
        rel = p.relative_to(root).as_posix()
        if schema in PRODUCTION:
            found.setdefault(schema, []).append(rel)
    return found


def _reject_identifier(obj):
    """Returns the negative fixture's declared failure identifier (its
    `code` for DForward/DMap-2-shaped fixtures, or `case` for the relational
    DDeploy/2-shaped ones), or None if `x_expect_reject` is missing/malformed."""
    block = obj.get("x_expect_reject")
    if not isinstance(block, dict):
        return None
    ident = block.get("code") or block.get("case")
    return ident if isinstance(ident, str) and ident.strip() else None


# `canonical-v1/negative/` is SHARED with pre-existing, unrelated Stage-110
# fixtures for datum.dgraph/1 / datum.dcontinuum/1 / datum.dmap/1 (e.g.
# `dmap.wrong-schema.json`, a DMap/1 negative, not a Phase 113A DForward
# one) -- a bare directory glob would wrongly pull those in. Filename
# PREFIX, not directory alone, is what actually identifies a fixture's
# schema family here.
FAMILY_FILENAME_PREFIXES = {
    "datum.dforward/1": "dforward.",
    "datum.dmap/2": "dmap.",
    "datum.ddeploy-proposal/2": "proposal.",
    "datum.ddeploy-acceptance/2": "acceptance.",
}


def negative_fixture_paths(root=ROOT):
    """Every `*.json` file under a known family's `negative/` directory
    whose filename prefix identifies it as belonging to that schema,
    discovered by LOCATION+NAME rather than by its own `schema` field -- a
    `wrong-schema` negative fixture deliberately carries an incorrect
    `schema` value (e.g. `datum.dmap/1` instead of `datum.dmap/2`), so
    `schema_locations()`'s schema-value-based discovery would silently miss
    it entirely. This is what makes wrong-schema negatives visible to the
    checker at all, without also pulling in unrelated pre-existing
    Stage-110 fixtures that happen to share the same `negative/` directory."""
    paths = []
    for schema, prefix in FAMILY_FILENAME_PREFIXES.items():
        neg_dir = root / ALLOWED[schema] / "negative"
        if not neg_dir.is_dir():
            continue
        for p in neg_dir.glob(f"{prefix}*.json"):
            paths.append(p.relative_to(root).as_posix())
    return paths


def discover_all(root=ROOT):
    """Union of schema-value-discovered fixtures (`schema_locations`) and
    filename+location-discovered negatives (`negative_fixture_paths`) -- the
    latter is what makes wrong-schema negative fixtures visible at all,
    since they deliberately carry an incorrect `schema` field. Each path
    from `negative_fixture_paths` is already unambiguously tagged with its
    real schema (by which prefix matched it), so no field-based
    disambiguation is needed here."""
    found = schema_locations(root)
    for schema, prefix in FAMILY_FILENAME_PREFIXES.items():
        neg_dir = root / ALLOWED[schema] / "negative"
        if not neg_dir.is_dir():
            continue
        bucket = found.setdefault(schema, [])
        for p in neg_dir.glob(f"{prefix}*.json"):
            rel = p.relative_to(root).as_posix()
            if rel not in bucket:
                bucket.append(rel)
    return found


def validate(root=ROOT):
    found = discover_all(root)
    errors = []

    # Schema-family ownership: unique by WIRE FAMILY, not file count. A
    # schema ID may appear in many fixtures (positive and negative) inside
    # its own legitimate family directory; it must never be claimed by a
    # foreign family directory, including under a `rogue/` or
    # `rogue/negative/` path.
    for schema, paths in found.items():
        for rel in paths:
            if ALLOWED[schema] not in rel.split("/"):
                errors.append(f"{schema}: non-canonical fixture family ownership: {rel}")

    for schema, family in ALLOWED.items():
        if not any(family in p.split("/") and "/negative/" not in p for p in found.get(schema, [])):
            errors.append(f"missing positive fixture for {schema}")

    for schema, paths in found.items():
        for rel in paths:
            obj = json.loads((root / rel).read_text(encoding="utf-8"))
            is_negative = "/negative/" in rel

            if not is_negative:
                for key in REQUIRED_FIELDS[schema]:
                    if key not in obj:
                        errors.append(f"{rel}: missing {schema} field {key}")
                if schema == "datum.dforward/1":
                    for ep in obj.get("external_endpoints", []):
                        if ep.get("direction") not in ("input", "output"):
                            errors.append(f"{rel}: invalid endpoint direction")
                continue

            # Negative fixture: must declare what it expects to fail with.
            ident = _reject_identifier(obj)
            if ident is None:
                errors.append(f"{rel}: negative fixture must carry a non-empty x_expect_reject.code or x_expect_reject.case")
                continue

            # Cheap, independently-implemented structural spot-checks for a
            # couple of failure shapes that are trivial to verify here
            # without re-implementing the real validator. Anything not
            # covered by these two shapes is left entirely to the Rust
            # tests, which already exercise every negative fixture's real
            # rejection against the actual production code.
            if ident == "schema_invalid":
                family_schema = next((s for s, fam in ALLOWED.items() if fam == family_of(rel)), None)
                if family_schema is not None and obj.get("schema") == family_schema:
                    errors.append(f"{rel}: claims schema_invalid but its own schema field is already the correct production schema")
            elif ident == "deny_unknown_fields":
                rogue_keys = set(obj.keys()) - KNOWN_KEYS
                if not rogue_keys:
                    errors.append(f"{rel}: claims deny_unknown_fields rejection but carries no top-level key outside the known field set {sorted(KNOWN_KEYS)}")

    return errors


def family_of(rel):
    parts = rel.split("/")
    return parts[0] if parts else ""


def main():
    errors = validate()
    if errors:
        raise SystemExit("\n".join(errors))
    locs = discover_all()
    total = sum(map(len, locs.values()))
    positives = sum(1 for paths in locs.values() for p in paths if "/negative/" not in p)
    negatives = total - positives
    print(f"phase113a fixture/schema checks passed ({total} production fixtures: {positives} positive, {negatives} negative)")


if __name__ == "__main__":
    main()
