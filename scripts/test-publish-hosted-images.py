#!/usr/bin/env python3
"""Exercise publisher failure boundaries with fake Docker and real jq, offline."""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
PUBLISHER = ROOT / "scripts/publish-hosted-images.sh"
IMAGE = "ghcr.io/infimount/infimount-hosted"
REVISION = "a" * 40
INDEX = "sha256:" + "e" * 64
WRONG = "sha256:" + "f" * 64
CONFIG = {"amd64": "sha256:" + "a" * 64, "arm64": "sha256:" + "b" * 64}
MANIFEST = {"amd64": "sha256:" + "c" * 64, "arm64": "sha256:" + "d" * 64}

# This executable handles every Docker invocation and never delegates to a
# daemon or network. Unsupported commands fail, rather than becoming no-ops.
# jq remains real so its validation predicates are exercised, not reimplemented.
FAKE_DOCKER = r'''
import json
import os
from pathlib import Path
import sys

state_path = Path(os.environ["PUBLISH_FIXTURE_STATE"])
state = json.loads(state_path.read_text())
args = sys.argv[1:]
with Path(os.environ["PUBLISH_FIXTURE_LOG"]).open("a") as log:
    log.write(json.dumps(args) + "\n")
image = "ghcr.io/infimount/infimount-hosted"
tag = os.environ["GITHUB_REF_NAME"]

def save():
    state_path.write_text(json.dumps(state))

def fail(message):
    print(message, file=sys.stderr)
    raise SystemExit(1)

def index():
    result = {
        "digest": state["index_digest"],
        "manifests": [
            {"platform": {"os": "linux", "architecture": arch}, "digest": row["manifest_digest"]}
            for arch, row in state["platforms"].items()
        ],
    }
    if state.get("bad_index_platform"):
        result["manifests"][1]["platform"]["architecture"] = "amd64"
    return result

if args[:2] == ["load", "--input"] and len(args) == 3:
    arch = Path(args[2]).name.removesuffix(".tar.gz")
    if arch not in state["platforms"]:
        fail("unknown input archive")
    state["platforms"][arch]["loaded"] = True
    save()
elif args[:2] == ["image", "inspect"] and len(args) == 5:
    arch = args[2].removeprefix("infimount-hosted:qualification-")
    row = state["platforms"].get(arch)
    if not row or not row.get("loaded") or args[3:] != ["--format", "{{.Id}}"]:
        fail("unexpected local image inspection")
    print(row.get("loaded_digest", row["config_digest"]))
elif args[:1] == ["tag"] and len(args) == 3:
    pass
elif args[:1] == ["push"] and len(args) == 2:
    if state.get("push_failure"):
        fail("synthetic push failure")
elif args[:3] == ["buildx", "imagetools", "create"]:
    if len(args) < 6 or args[3] != "--tag":
        fail("unexpected manifest creation")
    target, sources = args[4], args[5:]
    if target.endswith("-index"):
        expected = [image + "@" + row["manifest_digest"] for row in state["platforms"].values()]
        if sources != expected:
            fail("combined manifest sources changed")
        state["staged"] = True
    elif target == image + ":" + tag:
        if not state.get("staged") or sources != [image + "@" + state["index_digest"]]:
            fail("promotion did not use the staged digest")
        state["promoted"] = True
    else:
        fail("unexpected manifest target")
    save()
elif args[:3] == ["buildx", "imagetools", "inspect"] and len(args) >= 5:
    ref = args[3]
    if args[4:] == ["--raw"]:
        for row in state["platforms"].values():
            if ref == image + "@" + row["manifest_digest"]:
                print(json.dumps({"config": {"digest": row.get("remote_config_digest", row["config_digest"])}}))
                break
        else:
            fail("unknown raw manifest digest")
    elif args[4:] == ["--format", "{{json .Manifest}}"]:
        if ref == image + ":" + tag:
            if state.get("promoted") or state["existing"] == "same":
                result = index()
                if state.get("post_promote_mismatch"):
                    result["digest"] = "sha256:" + "f" * 64
                print(json.dumps(result))
            elif state["existing"] == "different":
                result = index()
                result["digest"] = "sha256:" + "f" * 64
                print(json.dumps(result))
            elif state["existing"] == "absent":
                fail("ERROR: " + ref + ": not found")
            elif state["existing"] == "absent_plain":
                fail(ref + ": not found")
            elif state["existing"] == "credentials_not_found":
                fail("ERROR: error getting credentials: credentials not found in native keychain")
            elif state["existing"] == "wrong_ref_not_found":
                fail("ERROR: ghcr.io/another/image:v9.9.9: not found")
            elif state["existing"] == "repository_not_found":
                fail("ERROR: unauthorized: repository not found")
            elif state["existing"] == "network":
                fail("ERROR: context deadline exceeded")
            else:
                fail("unexpected existing-ref fixture")
        elif ref == image + "@" + state["index_digest"] or ref.endswith("-index"):
            if not state.get("staged"):
                fail("staged manifest does not exist")
            print(json.dumps(index()))
        else:
            for arch, row in state["platforms"].items():
                if ref.endswith("-" + arch) and ref.startswith(image + ":build-"):
                    print(json.dumps({"digest": row["manifest_digest"]}))
                    break
            else:
                fail("unknown manifest reference")
    else:
        fail("unexpected manifest inspection format")
else:
    fail("unsupported fake Docker invocation: " + repr(args))
'''


class Fixture:
    def __init__(self, root: Path, tag="v0.8.2"):
        self.root = root
        self.images = root / "images"
        self.assets = root / "assets"
        self.bin = root / "bin"
        for directory in (self.images, self.assets, self.bin):
            directory.mkdir()
        self.state_path = root / "state.json"
        self.log_path = root / "docker.jsonl"
        self.state = {
            "index_digest": INDEX,
            "existing": "absent",
            "staged": False,
            "promoted": False,
            "platforms": {
                arch: {"config_digest": CONFIG[arch], "manifest_digest": MANIFEST[arch]}
                for arch in ("amd64", "arm64")
            },
        }
        self.save()
        for arch in CONFIG:
            (self.images / f"{arch}.tar.gz").write_bytes(b"offline image fixture")
            self.write_inspection(arch, {
                "Id": CONFIG[arch], "Architecture": arch, "Os": "linux",
                "Config": {"User": "65532:65532", "Labels": {
                    "org.opencontainers.image.revision": REVISION,
                    "org.opencontainers.image.version": tag.removeprefix("v"),
                    "org.opencontainers.image.source": "https://github.com/infimount/infimount",
                }},
            })
        docker = self.bin / "docker"
        docker.write_text(f"#!{sys.executable}\n" + FAKE_DOCKER)
        docker.chmod(0o755)
        self.env = {
            "PATH": str(self.bin) + os.pathsep + str(Path(shutil.which("jq")).parent) + os.pathsep + os.defpath,
            "GITHUB_REF_NAME": tag, "GITHUB_SHA": REVISION,
            "GITHUB_REPOSITORY": "infimount/infimount", "GITHUB_REF_TYPE": "tag",
            "GITHUB_RUN_ID": "1234", "GITHUB_RUN_ATTEMPT": "2",
            "GITHUB_OUTPUT": str(root / "outputs"),
            "PUBLISH_FIXTURE_STATE": str(self.state_path), "PUBLISH_FIXTURE_LOG": str(self.log_path),
        }

    def save(self):
        self.state_path.write_text(json.dumps(self.state))

    def reload(self):
        self.state = json.loads(self.state_path.read_text())

    def inspection(self, arch="amd64"):
        return json.loads((self.images / f"{arch}.inspect.json").read_text())[0]

    def write_inspection(self, arch, value):
        (self.images / f"{arch}.inspect.json").write_text(json.dumps([value]))

    def calls(self):
        return [json.loads(line) for line in self.log_path.read_text().splitlines()] if self.log_path.exists() else []

    def promotions(self):
        return [call for call in self.calls() if call[:4] == ["buildx", "imagetools", "create", "--tag"]
                and call[4] == IMAGE + ":" + self.env["GITHUB_REF_NAME"]]

    def run(self, mode):
        result = subprocess.run([shutil.which("bash"), str(PUBLISHER), mode, str(self.images), str(self.assets)],
                                env=self.env, capture_output=True, text=True, timeout=15)
        self.reload()
        return result

    def stage(self):
        result = self.run("stage")
        if result.returncode:
            raise AssertionError(result.stdout + result.stderr)
        return json.loads((self.assets / "hosted-image.json").read_text())


class PublisherTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="infimount-publish-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def fixture(self, name="case", tag="v0.8.2"):
        path = self.root / name
        path.mkdir()
        return Fixture(path, tag)

    def assert_rejected(self, fixture, mode):
        result = fixture.run(mode)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(fixture.promotions(), "rejected input must not promote a release tag")

    def test_stage_records_exact_source_and_digests_without_publishing_a_version(self):
        fixture = self.fixture()
        receipt = fixture.stage()
        self.assertEqual(receipt["source_commit"], REVISION)
        self.assertEqual(receipt["digest"], INDEX)
        self.assertEqual(receipt["tag"], "v0.8.2")
        self.assertEqual({row["platform"] for row in receipt["platforms"]}, {"linux/amd64", "linux/arm64"})
        for row in receipt["platforms"]:
            arch = row["platform"].split("/")[1]
            self.assertEqual(row["config_digest"], CONFIG[arch])
            self.assertEqual(row["digest"], MANIFEST[arch])
        self.assertFalse(fixture.promotions())
        self.assertFalse(any(IMAGE + ":v0.8.2" in call for call in fixture.calls()))
        self.assertFalse(any(":latest" in argument for call in fixture.calls() for argument in call))

    def test_explicit_promotion_copies_only_the_staged_digest(self):
        for index, missing in enumerate(("absent", "absent_plain")):
            with self.subTest(missing=missing):
                fixture = self.fixture(str(index))
                fixture.stage()
                fixture.state["existing"] = missing
                fixture.save()
                result = fixture.run("promote")
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(len(fixture.promotions()), 1)
                self.assertEqual(fixture.promotions()[0][5:], [IMAGE + "@" + INDEX])

    def test_existing_exact_version_is_idempotent_and_different_version_is_refused(self):
        for existing, success in (("same", True), ("different", False)):
            with self.subTest(existing=existing):
                fixture = self.fixture(existing)
                fixture.stage()
                fixture.state["existing"] = existing
                fixture.save()
                result = fixture.run("promote")
                self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
                self.assertFalse(fixture.promotions())

    def test_credentials_repository_and_network_errors_are_never_missing_manifests(self):
        for mode in ("credentials_not_found", "wrong_ref_not_found", "repository_not_found", "network"):
            with self.subTest(mode=mode):
                fixture = self.fixture(mode)
                fixture.stage()
                fixture.state["existing"] = mode
                fixture.save()
                self.assert_rejected(fixture, "promote")

    def test_wrong_context_is_rejected_before_any_docker_command(self):
        for index, (key, value) in enumerate((
            ("GITHUB_REPOSITORY", "someone/fork"), ("GITHUB_REF_TYPE", "branch"),
            ("GITHUB_REF_NAME", "main"), ("GITHUB_REF_NAME", "v0.8.2;echo bad"),
            ("GITHUB_SHA", "not-a-commit"),
        )):
            with self.subTest(key=key, value=value):
                fixture = self.fixture(str(index))
                fixture.env[key] = value
                self.assert_rejected(fixture, "stage")
                self.assertEqual(fixture.calls(), [])

    def test_unqualified_local_metadata_and_missing_archives_stop_staging(self):
        for index, (field, value) in enumerate((
            ("Architecture", "arm64"), ("Os", "windows"), ("Id", "not-a-digest"),
            ("User", "root"), ("revision", "b" * 40), ("version", "9.9.9"),
            ("source", "https://github.com/someone/another-repo"),
        )):
            with self.subTest(field=field):
                fixture = self.fixture(str(index))
                inspection = fixture.inspection()
                if field == "User":
                    inspection["Config"]["User"] = value
                elif field in ("revision", "version", "source"):
                    inspection["Config"]["Labels"]["org.opencontainers.image." + field] = value
                else:
                    inspection[field] = value
                fixture.write_inspection("amd64", inspection)
                self.assert_rejected(fixture, "stage")
                self.assertFalse((fixture.assets / "hosted-image.json").exists())
        fixture = self.fixture("missing-arm64")
        (fixture.images / "arm64.tar.gz").unlink()
        self.assert_rejected(fixture, "stage")

    def test_loaded_bytes_remote_configuration_and_combined_platforms_are_checked(self):
        for mutation in ("loaded_digest", "remote_config_digest", "manifest_digest", "bad_index_platform"):
            with self.subTest(mutation=mutation):
                fixture = self.fixture(mutation)
                if mutation == "bad_index_platform":
                    fixture.state[mutation] = True
                else:
                    fixture.state["platforms"]["amd64"][mutation] = "invalid" if mutation == "manifest_digest" else WRONG
                fixture.save()
                self.assert_rejected(fixture, "stage")
                self.assertFalse((fixture.assets / "hosted-image.json").exists())

    def test_changed_receipt_identity_and_platform_members_cannot_be_promoted(self):
        for index, (field, value) in enumerate((
            ("schema_version", 2), ("tag", "v9.9.9"), ("source_commit", "b" * 40),
            ("image", "ghcr.io/someone/another-image"), ("digest", WRONG), ("platforms", []),
        )):
            with self.subTest(field=field):
                fixture = self.fixture(str(index))
                receipt = fixture.stage()
                receipt[field] = value
                (fixture.assets / "hosted-image.json").write_text(json.dumps(receipt))
                self.assert_rejected(fixture, "promote")

    def test_promotion_postcheck_detects_a_changed_registry_digest(self):
        fixture = self.fixture()
        fixture.stage()
        fixture.state["post_promote_mismatch"] = True
        fixture.save()
        result = fixture.run("promote")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(fixture.promotions()), 1)
        self.assertNotIn("Promoted validated release image", result.stdout)

    def test_candidate_uses_its_exact_tag_and_never_a_stable_or_latest_alias(self):
        fixture = self.fixture(tag="v0.8.3-rc.1")
        receipt = fixture.stage()
        self.assertEqual(receipt["tag"], "v0.8.3-rc.1")
        result = fixture.run("promote")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(fixture.promotions()[0][4], IMAGE + ":v0.8.3-rc.1")
        self.assertFalse(any(argument.endswith((":latest", ":v0.8.3")) for call in fixture.calls() for argument in call))


if __name__ == "__main__":
    if not shutil.which("jq") or not shutil.which("bash"):
        raise SystemExit("These offline tests require jq and bash. Docker is fully mocked.")
    unittest.main(verbosity=2)
