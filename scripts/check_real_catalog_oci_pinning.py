#!/usr/bin/env python3
"""D-Node/D-Continuum + 107A/107B foundation closure (Subtranche B3/B8):
real catalog OCI-pinning completeness checker.

Scans every real, checked-in `artifacts/catalog/*.json` ServiceArtifact
(excluding `scenario.json`, which is a scenario/topology descriptor, not a
ServiceArtifact) and asserts:

  - every `image_source_kind: "registry"` container artifact's `image` is
    digest-pinned (`repository@sha256:<64 lowercase hex>`), NEVER a bare
    mutable tag;
  - every `image_source_kind: "local_build"` container artifact is reported
    separately as PERMANENTLY unpinnable in this slice (ADR-0013 Sec. 3 --
    `pin_image_identity` refuses `LocalBuildNotResolvable` by design; no
    invented digest is ever acceptable here);
  - `image_source_kind: "legacy_unclassified"` (if any) is reported as a
    hard FAILURE -- every real artifact must explicitly classify this field
    (ADR-0013 v3).

This mirrors, at the catalog-file level, exactly what
`core::service_artifact_registry::catalog_inventory_classifies_every_real_artifact_explicitly`
already proves for `image_source_kind` classification alone; this script
additionally checks the DIGEST-PINNING content itself, which that Rust test
does not.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
CATALOG_DIR = REPO_ROOT / "artifacts" / "catalog"
NON_ARTIFACT_FILES = {"scenario.json"}

DIGEST_PIN_RE = re.compile(r"^[^@]+@sha256:[0-9a-f]{64}$")


def classify(path: Path) -> dict:
    data = json.loads(path.read_text(encoding="utf-8"))
    runtime = data.get("runtime", {})
    if runtime.get("kind") != "container":
        return {"path": path.name, "kind": "non_container"}

    container = runtime.get("container", {})
    image = container.get("image", "")
    source_kind = container.get("image_source_kind")
    pinned = bool(DIGEST_PIN_RE.match(image))
    return {
        "path": path.name,
        "kind": "container",
        "image": image,
        "image_source_kind": source_kind,
        "pinned": pinned,
    }


def main() -> int:
    if not CATALOG_DIR.is_dir():
        print(f"FAIL: catalog directory not found: {CATALOG_DIR}")
        return 2

    entries = [
        classify(p)
        for p in sorted(CATALOG_DIR.glob("*.json"))
        if p.name not in NON_ARTIFACT_FILES
    ]

    registry_pinned = [e for e in entries if e["kind"] == "container" and e["image_source_kind"] == "registry" and e["pinned"]]
    registry_unpinned = [e for e in entries if e["kind"] == "container" and e["image_source_kind"] == "registry" and not e["pinned"]]
    local_build = [e for e in entries if e["kind"] == "container" and e["image_source_kind"] == "local_build"]
    legacy_unclassified = [e for e in entries if e["kind"] == "container" and e["image_source_kind"] not in ("registry", "local_build")]
    non_container = [e for e in entries if e["kind"] != "container"]

    print(f"total real catalog artifacts: {len(entries)}")
    print(f"  registry, digest-pinned:    {len(registry_pinned)}")
    print(f"  registry, UNPINNED:         {len(registry_unpinned)}")
    print(f"  local_build (never pinnable in this slice): {len(local_build)}")
    print(f"  legacy_unclassified (must be zero):          {len(legacy_unclassified)}")
    print(f"  non-container:              {len(non_container)}")

    ok = True
    if registry_unpinned:
        ok = False
        print("\nFAIL: registry-classified artifacts still using a mutable tag:")
        for e in registry_unpinned:
            print(f"  - {e['path']}: image={e['image']!r}")
    if legacy_unclassified:
        ok = False
        print("\nFAIL: artifacts missing an explicit image_source_kind classification:")
        for e in legacy_unclassified:
            print(f"  - {e['path']}: image_source_kind={e['image_source_kind']!r}")

    if local_build:
        print("\nINFO: local_build artifacts (permanently unpinnable pending a future")
        print("      governed-build-attestation slice, ADR-0013 Sec. 3 -- NOT a failure):")
        for e in local_build:
            print(f"  - {e['path']}: image={e['image']!r}")

    if ok:
        print("\nOK: every registry-classified real catalog artifact is digest-pinned")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
