#!/usr/bin/env python3
"""Network-free release inventory rejection tests. Native execution is a CI gate."""
import importlib.util
import io
import json
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

spec = importlib.util.spec_from_file_location("distribution", Path(__file__).with_name("check-distribution-assets.py"))
distribution = importlib.util.module_from_spec(spec)
spec.loader.exec_module(distribution)


class DistributionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.version = "0.8.2-rc.1"
        self.metadata = "Metadata-Version: 2.4\nName: infimount\nVersion: 0.8.2rc1\nRequires-Python: >=3.10\n\n"
        self.tags = ["manylinux_2_28_x86_64", "manylinux_2_28_aarch64", "macosx_11_0_x86_64", "macosx_11_0_arm64", "win_amd64"]
        for tag in self.tags:
            self.write_wheel(tag)
        with tarfile.open(self.directory / "infimount-0.8.2rc1.tar.gz", "w:gz") as archive:
            for name, data in [("PKG-INFO", self.metadata.encode()), ("Cargo.lock", b"version = 4\n")]:
                member = tarfile.TarInfo(f"infimount-0.8.2rc1/{name}")
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))
        self.image = {"schema_version": 1, "tag": "v0.8.2-rc.1", "source_commit": "a" * 40, "image": "ghcr.io/infimount/infimount-hosted", "digest": "sha256:" + "b" * 64, "platforms": [{"platform": "linux/" + arch, "digest": "sha256:" + str(number) * 64, "config_digest": "sha256:" + str(number + 2) * 64} for number, arch in enumerate(["amd64", "arm64"], 1)]}
        self.write_image()

    def write_image(self):
        (self.directory / "hosted-image.json").write_text(json.dumps(self.image))

    def write_wheel(self, tag, metadata=None, extra=None):
        path = self.directory / f"infimount-0.8.2rc1-cp310-abi3-{tag}.whl"
        with zipfile.ZipFile(path, "w") as archive:
            archive.writestr("infimount/__init__.py", "# test fixture\n")
            archive.writestr("infimount/_native.abi3.so", b"fixture, never executed")
            archive.writestr("infimount-0.8.2rc1.dist-info/METADATA", metadata or self.metadata)
            archive.writestr("infimount-0.8.2rc1.dist-info/WHEEL", f"Wheel-Version: 1.0\nTag: cp310-abi3-{tag}\n\n")
            if extra:
                archive.writestr(extra, "unsafe")
        return path

    def test_complete_inventory_and_prerelease_normalization(self):
        files = distribution.check_python(self.directory, self.version)
        self.assertEqual(len(files), 6)
        self.assertEqual({item["platform"] for item in files}, distribution.PLATFORMS | {"source"})
        distribution.check_image(self.directory, "v" + self.version, "a" * 40)

    def test_missing_architecture_fails(self):
        next(self.directory.glob("*aarch64.whl")).unlink()
        with self.assertRaisesRegex(ValueError, "five native wheels"):
            distribution.check_python(self.directory, self.version)

    def test_filename_cannot_hide_wrong_metadata_version(self):
        self.write_wheel(self.tags[0], self.metadata.replace("0.8.2rc1", "0.8.2"))
        with self.assertRaisesRegex(ValueError, "metadata disagrees"):
            distribution.check_python(self.directory, self.version)

    def test_wrong_abi_and_unsafe_archive_names_fail(self):
        path = next(self.directory.glob("*win_amd64.whl"))
        path.rename(path.with_name(path.name.replace("cp310-abi3", "cp314-cp314t")))
        with self.assertRaisesRegex(ValueError, "wheel name or ABI"):
            distribution.check_python(self.directory, self.version)
        next(self.directory.glob("*cp314t*.whl")).unlink()
        self.write_wheel("win_amd64", extra="../escape")
        with self.assertRaisesRegex(ValueError, "unsafe wheel"):
            distribution.check_python(self.directory, self.version)

    def test_wrong_source_commit_image_repo_and_architecture_fail(self):
        for field, wrong, expected in [("source_commit", "c" * 40, "source mismatch"), ("image", "ghcr.io/unrelated/image", "repository mismatch"), ("platforms", self.image["platforms"][:1], "architecture coverage")]:
            original = self.image[field]
            self.image[field] = wrong
            self.write_image()
            with self.assertRaisesRegex(ValueError, expected):
                distribution.check_image(self.directory, "v" + self.version, "a" * 40)
            self.image[field] = original


if __name__ == "__main__":
    unittest.main()
