#!/usr/bin/env python3
"""Selftest for check_real_catalog_oci_pinning.py -- exercises the pure
classification/regex logic against synthetic fixtures (never touches the
real `artifacts/catalog/` tree, so it cannot regress silently just because
the real catalog changes)."""
import importlib.util
import sys
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent / "check_real_catalog_oci_pinning.py"
spec = importlib.util.spec_from_file_location("check_real_catalog_oci_pinning", SCRIPT_PATH)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)  # type: ignore[union-attr]


class DigestPinRegexTests(unittest.TestCase):
    def test_pinned_reference_matches(self):
        self.assertTrue(checker.DIGEST_PIN_RE.match("docker.io/library/redis@sha256:" + "a" * 64))

    def test_bare_tag_does_not_match(self):
        self.assertFalse(checker.DIGEST_PIN_RE.match("redis:7-alpine"))

    def test_tag_plus_digest_still_matches_since_repository_is_greedy_up_to_at(self):
        # "repo:tag@sha256:<64 hex>" is still an acceptable pinned form --
        # the digest is what governs identity regardless of a co-present tag.
        self.assertTrue(checker.DIGEST_PIN_RE.match("redis:7-alpine@sha256:" + "b" * 64))

    def test_wrong_length_digest_does_not_match(self):
        self.assertFalse(checker.DIGEST_PIN_RE.match("redis@sha256:" + "a" * 63))

    def test_uppercase_hex_does_not_match(self):
        self.assertFalse(checker.DIGEST_PIN_RE.match("redis@sha256:" + "A" * 64))

    def test_missing_sha256_prefix_does_not_match(self):
        self.assertFalse(checker.DIGEST_PIN_RE.match("redis@" + "a" * 64))


class ClassifyTests(unittest.TestCase):
    def setUp(self):
        import json
        import tempfile

        self.tmpdir = Path(tempfile.mkdtemp())

    def write(self, name: str, body: dict) -> Path:
        import json

        path = self.tmpdir / name
        path.write_text(json.dumps(body), encoding="utf-8")
        return path

    def test_registry_pinned_artifact(self):
        path = self.write("a.json", {
            "runtime": {"kind": "container", "container": {
                "image": "docker.io/library/redis@sha256:" + "a" * 64,
                "image_source_kind": "registry",
            }},
        })
        result = checker.classify(path)
        self.assertEqual(result["kind"], "container")
        self.assertTrue(result["pinned"])
        self.assertEqual(result["image_source_kind"], "registry")

    def test_registry_unpinned_artifact(self):
        path = self.write("b.json", {
            "runtime": {"kind": "container", "container": {
                "image": "redis:7-alpine",
                "image_source_kind": "registry",
            }},
        })
        result = checker.classify(path)
        self.assertFalse(result["pinned"])

    def test_local_build_artifact(self):
        path = self.write("c.json", {
            "runtime": {"kind": "container", "container": {
                "image": "local/thing:dev",
                "image_source_kind": "local_build",
            }},
        })
        result = checker.classify(path)
        self.assertEqual(result["image_source_kind"], "local_build")
        self.assertFalse(result["pinned"])

    def test_non_container_artifact(self):
        path = self.write("d.json", {"runtime": {"kind": "native_process"}})
        result = checker.classify(path)
        self.assertEqual(result["kind"], "non_container")


class RealCatalogIntegrationTest(unittest.TestCase):
    """The one test that DOES touch the real tree -- proves the checker's
    main() currently reports a clean, fully-pinned real catalog."""

    def test_real_catalog_currently_passes(self):
        exit_code = checker.main()
        self.assertEqual(exit_code, 0, "the real, checked-in artifacts/catalog/ tree must currently pass this checker")


if __name__ == "__main__":
    unittest.main()
