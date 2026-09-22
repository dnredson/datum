"""Selftest for `check_phase113a_contracts.py`.

Independently injects rogue fixture ownership for ALL FOUR production
schemas, including cases placed under `rogue/negative/` (not just a bare
`rogue/` directory) -- proving a rogue fixture cannot escape the
schema-family-ownership check merely by imitating a legitimate `negative/`
layout.
"""
from pathlib import Path
import tempfile, json
from check_phase113a_contracts import validate

BASE_FIXTURES = {
    "datum.dforward/1": {
        "project_id": "p", "dforward_id": "f", "revision": 1,
        "diots": [], "external_endpoints": [], "chains": [], "support_jobs": [],
    },
    "datum.dmap/2": {
        "project_id": "p", "application_id": "a", "dmap_id": "m", "revision": 1,
        "dgraph_ref": {}, "dcontinuum_ref": {}, "dforward_ref": {},
        "placements": [], "diot_placements": [], "dcall_realizations": [], "external_port_bindings": [],
    },
    "datum.ddeploy-proposal/2": {
        "proposal_id": "p", "project_id": "p", "application_id": "a",
        "source": "operator", "created_at_utc": "2026-01-01T00:00:00Z",
        "dgraph_ref": {}, "dcontinuum_ref": {}, "dforward_ref": {},
        "placements": [], "diot_placements": [], "dcall_realizations": [], "external_port_bindings": [],
        "diot_artifact_authority": {},
    },
    "datum.ddeploy-acceptance/2": {
        "acceptance_id": "a", "proposal_id": "p", "proposal_digest": "sha256:" + "0" * 64,
        "project_id": "p", "application_id": "a", "accepted_at_utc": "now", "accepted_by": "operator",
        "materialized_dmap_ref": {},
    },
}
FAMILY_OF = {
    "datum.dforward/1": "canonical-v1",
    "datum.dmap/2": "dmap-v2",
    "datum.ddeploy-proposal/2": "ddeploy-v2",
    "datum.ddeploy-acceptance/2": "ddeploy-v2",
}


def _write_valid_tree(root):
    for schema, obj in BASE_FIXTURES.items():
        family = FAMILY_OF[schema]
        target = root / family
        target.mkdir(exist_ok=True)
        payload = dict(obj)
        payload["schema"] = schema
        (target / (schema.replace("/", "-") + ".json")).write_text(json.dumps(payload), encoding="utf-8")


def test_clean_tree_passes():
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        _write_valid_tree(root)
        assert not validate(root), validate(root)


def test_rogue_ownership_is_rejected_for_every_production_schema():
    # For EACH of the four production schemas independently: a genuinely
    # valid tree, PLUS one rogue fixture claiming that exact schema from a
    # foreign, non-canonical directory. Every one must be rejected on its
    # own -- not just the one schema the original selftest happened to
    # cover.
    for schema in BASE_FIXTURES:
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write_valid_tree(root)
            (root / "rogue").mkdir()
            rogue = dict(BASE_FIXTURES[schema])
            rogue["schema"] = schema
            (root / "rogue" / "rogue.json").write_text(json.dumps(rogue), encoding="utf-8")
            errors = validate(root)
            assert errors, f"rogue top-level ownership of {schema} was not rejected"


def test_rogue_ownership_under_a_negative_directory_is_rejected_for_every_production_schema():
    # The stronger, previously-missing case: a rogue fixture placed under
    # `rogue/negative/` (mimicking the legitimate `<family>/negative/`
    # layout) must not escape the family-ownership check merely because it
    # LOOKS like a negative fixture directory. Give it a well-formed
    # `x_expect_reject` block too, so it cannot be rejected for the wrong
    # reason (missing metadata) -- it must be caught specifically for
    # claiming a foreign family.
    for schema in BASE_FIXTURES:
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            _write_valid_tree(root)
            neg_dir = root / "rogue" / "negative"
            neg_dir.mkdir(parents=True)
            rogue = dict(BASE_FIXTURES[schema])
            rogue["schema"] = schema
            rogue["x_expect_reject"] = {"code": "schema_invalid", "case": "schema_invalid"}
            (neg_dir / "rogue-negative.json").write_text(json.dumps(rogue), encoding="utf-8")
            errors = validate(root)
            assert errors, f"rogue rogue/negative/ ownership of {schema} was not rejected"


def test_missing_x_expect_reject_on_a_real_negative_fixture_is_rejected():
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        _write_valid_tree(root)
        neg_dir = root / "dmap-v2" / "negative"
        neg_dir.mkdir()
        obj = dict(BASE_FIXTURES["datum.dmap/2"])
        obj["schema"] = "datum.dmap/1"  # deliberately wrong, i.e. a genuine wrong-schema shape
        (neg_dir / "dmap.no-metadata.json").write_text(json.dumps(obj), encoding="utf-8")
        errors = validate(root)
        assert errors, "a negative-shaped fixture with no x_expect_reject metadata was not rejected"


def test_a_vs1_located_fixture_cannot_reclaim_the_production_dforward_schema():
    # The historical Stage-107 sketch is relabelled
    # `datum.x-fixture.vs1-dforward/1` and lives under `vs1/` -- it must
    # never be able to claim the real production `datum.dforward/1` schema
    # ID. This proves the family-ownership check (not merely the Rust
    # `CanonicalDForwardV1`'s own schema string check) independently
    # enforces that boundary too.
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        _write_valid_tree(root)
        vs1_dir = root / "vs1"
        vs1_dir.mkdir()
        rogue_vs1 = dict(BASE_FIXTURES["datum.dforward/1"])
        rogue_vs1["schema"] = "datum.dforward/1"  # deliberately reclaiming the production ID
        (vs1_dir / "dforward.monitor-f49c8.json").write_text(json.dumps(rogue_vs1), encoding="utf-8")
        errors = validate(root)
        assert errors, "a vs1/-located fixture claiming the production datum.dforward/1 schema was not rejected"


def main():
    test_clean_tree_passes()
    test_rogue_ownership_is_rejected_for_every_production_schema()
    test_rogue_ownership_under_a_negative_directory_is_rejected_for_every_production_schema()
    test_missing_x_expect_reject_on_a_real_negative_fixture_is_rejected()
    test_a_vs1_located_fixture_cannot_reclaim_the_production_dforward_schema()
    print("phase113a checker selftest passed (4 production schemas x rogue-top-level + rogue-negative-directory injection + vs1 relabel boundary)")


if __name__ == "__main__":
    main()
