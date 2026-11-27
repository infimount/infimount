#!/usr/bin/env python3
"""Focused tests of sdist extraction, lock provenance and reproducible packing."""
from __future__ import annotations

import copy
import importlib.util
import io
import os
from pathlib import Path
import tarfile
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("python_sdist_builder", Path(__file__).with_name("build-python-sdist.py"))
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


class SourceArchiveTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="infimount-sdist-test-")
        self.root = Path(self.temporary.name)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def archive(self, entries: list[tuple[str, bytes, bytes | None]]) -> Path:
        path = self.root / "input.tar.gz"
        with tarfile.open(path, "w:gz") as archive:
            for name, payload, kind in entries:
                member = tarfile.TarInfo(name)
                member.size = len(payload)
                if kind is not None:
                    member.type = kind
                    member.linkname = "../../outside"
                archive.addfile(member, io.BytesIO(payload))
        return path

    def test_rejects_traversal_links_aliases_and_multiple_roots(self) -> None:
        cases = [
            [("../outside", b"bad", None)],
            [("/absolute", b"bad", None)],
            [("C:/absolute", b"bad", None)],
            [(".", b"bad", None)],
            [("package/../outside", b"bad", None)],
            [("package\\outside", b"bad", None)],
            [("package/./alias", b"bad", None)],
            [("package/link", b"", tarfile.SYMTYPE)],
            [("package/link", b"", tarfile.LNKTYPE)],
            [("package/device", b"", tarfile.CHRTYPE)],
            [("package/file", b"one", None), ("package/file", b"two", None)],
            [("package/file", b"one", None), ("another/file", b"two", None)],
        ]
        for number, entries in enumerate(cases):
            with self.subTest(entries=entries), self.assertRaises(ValueError):
                builder.extract_source(self.archive(entries), self.root / f"unpacked-{number}")
        self.assertFalse((self.root / "outside").exists())

    def test_rejects_archive_limits_before_extracting_payload(self) -> None:
        archive = self.archive([("package/file", b"12345", None)])
        original = builder.MAX_UNPACKED_BYTES
        try:
            builder.MAX_UNPACKED_BYTES = 4
            with self.assertRaises(ValueError):
                builder.extract_source(archive, self.root / "size-limited")
        finally:
            builder.MAX_UNPACKED_BYTES = original

    def test_dependency_pruning_preserves_every_retained_identity_and_checksum(self) -> None:
        qualified = {
            "version": 4,
            "package": [
                {"name": "infimount_python", "version": "0.8.1", "dependencies": ["dependency 1.0.0"]},
                {"name": "dependency", "version": "1.0.0", "source": "registry+https://example.test/index", "checksum": "a" * 64},
                {"name": "unused", "version": "2.0.0", "source": "registry+https://example.test/index", "checksum": "b" * 64},
            ],
        }
        candidate = copy.deepcopy(qualified)
        candidate["package"].pop()
        candidate["package"][0]["dependencies"] = ["dependency"]
        proof = builder.verify_pruned_lock(qualified, candidate)
        self.assertEqual(proof["removed_packages"], 1)
        self.assertEqual(proof["unchanged_external_packages"], 1)
        for field, value in [("version", "1.0.1"), ("source", "git+https://example.test/replaced"), ("checksum", "c" * 64)]:
            changed = copy.deepcopy(candidate)
            changed["package"][1][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                builder.verify_pruned_lock(qualified, changed)
        replaced = copy.deepcopy(candidate)
        replaced["package"][1].pop("source")
        with self.assertRaises(ValueError):
            builder.verify_pruned_lock(qualified, replaced)

    def test_repacking_is_reproducible_despite_filesystem_timestamps(self) -> None:
        archive = self.archive([("package/README.md", b"readme", None), ("package/Cargo.lock", b"lock", None)])
        source = builder.extract_source(archive, self.root / "extracted")
        first = self.root / "first.tar.gz"
        second = self.root / "second.tar.gz"
        builder.pack_source(source, first, 123456789)
        for path in source.rglob("*"):
            os.utime(path, (999999999, 999999999))
        builder.pack_source(source, second, 123456789)
        self.assertEqual(first.read_bytes(), second.read_bytes())
        with tarfile.open(first) as verified:
            self.assertTrue(all(member.mtime == 123456789 and member.uid == member.gid == 0 for member in verified))

    def test_rejects_rewired_edges_even_when_both_package_versions_remain_qualified(self) -> None:
        registry = "registry+https://example.test/index"
        qualified = {
            "version": 4,
            "package": [
                {"name": "application", "version": "0.8.1", "dependencies": ["dependency 1.0.0"]},
                {"name": "dependency", "version": "1.0.0", "source": registry, "checksum": "a" * 64},
                {"name": "dependency", "version": "2.0.0", "source": registry, "checksum": "b" * 64},
                {"name": "unrelated", "version": "1.0.0", "source": registry, "checksum": "c" * 64},
            ],
        }
        for replacement in ["dependency 2.0.0", "unrelated", "dependency", "missing 1.0.0"]:
            candidate = copy.deepcopy(qualified)
            candidate["package"][0]["dependencies"] = [replacement]
            with self.subTest(replacement=replacement), self.assertRaises(ValueError):
                builder.verify_pruned_lock(qualified, candidate)
        # Removing the unused duplicate permits Cargo to shorten the surviving
        # edge, but its identity must still be the original 1.0.0 dependency.
        candidate = copy.deepcopy(qualified)
        candidate["package"] = candidate["package"][:2]
        candidate["package"][0]["dependencies"] = ["dependency"]
        proof = builder.verify_pruned_lock(qualified, candidate)
        self.assertEqual(proof["unchanged_dependency_edges"], 1)
        candidate["package"][0]["dependencies"] = []
        self.assertEqual(builder.verify_pruned_lock(qualified, candidate)["unchanged_dependency_edges"], 0)

    def test_source_qualified_edges_preserve_the_original_registry_or_git_identity(self) -> None:
        registry = "registry+https://example.test/index"
        other_registry = "registry+https://other.example.test/index"
        git = "git+https://example.test/dependency?branch=qualified"
        qualified = {
            "version": 4,
            "package": [
                {"name": "application", "version": "0.8.1", "dependencies": [f"dependency 1.0.0 ({registry})", f"git-dependency 1.0.0 ({git})"]},
                {"name": "dependency", "version": "1.0.0", "source": registry, "checksum": "a" * 64},
                {"name": "dependency", "version": "1.0.0", "source": other_registry, "checksum": "b" * 64},
                {"name": "git-dependency", "version": "1.0.0", "source": git + "#" + "1" * 40},
            ],
        }
        self.assertEqual(builder.verify_pruned_lock(qualified, qualified)["unchanged_dependency_edges"], 2)
        for replacement in [f"dependency 1.0.0 ({other_registry})", "dependency 1.0.0"]:
            candidate = copy.deepcopy(qualified)
            candidate["package"][0]["dependencies"][0] = replacement
            with self.subTest(replacement=replacement), self.assertRaises(ValueError):
                builder.verify_pruned_lock(qualified, candidate)
        candidate = copy.deepcopy(qualified)
        candidate["package"].pop(2)
        candidate["package"][0]["dependencies"] = ["dependency", "git-dependency"]
        self.assertEqual(builder.verify_pruned_lock(qualified, candidate)["unchanged_dependency_edges"], 2)

        # Two retained precise revisions with an omitted fragment cannot be
        # guessed. Qualified spelling must identify exactly one locked package.
        ambiguous = copy.deepcopy(qualified)
        ambiguous["package"].append({"name": "git-dependency", "version": "1.0.0", "source": git + "#" + "2" * 40})
        with self.assertRaises(ValueError):
            builder.verify_pruned_lock(ambiguous, ambiguous)


if __name__ == "__main__":
    unittest.main()
