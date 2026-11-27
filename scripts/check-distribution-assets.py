#!/usr/bin/env python3
"""Validate native wheel coverage and digest-pinned hosted release identity."""
from __future__ import annotations

import argparse
import email.parser
import hashlib
import json
import re
import tarfile
import zipfile
from pathlib import Path, PurePosixPath

PLATFORMS = {"linux-x86_64", "linux-aarch64", "macos-x86_64", "macos-arm64", "windows-amd64"}
DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def python_version(version):
    match = re.fullmatch(r"(\d+\.\d+\.\d+)(?:-(alpha|a|beta|b|rc|dev)\.(\d+))?", version)
    require(match is not None, "Python distribution requires a stable, alpha.N, beta.N, rc.N or dev.N version")
    core, pre, number = match.groups()
    return core if pre is None else core + {"alpha": "a", "a": "a", "beta": "b", "b": "b", "rc": "rc", "dev": ".dev"}[pre] + number


def safe_member(name):
    path = PurePosixPath(name)
    return not path.is_absolute() and ".." not in path.parts and "\\" not in name


def file_hash(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def wheel_platform(tag):
    if re.fullmatch(r"manylinux_2_28_x86_64(?:\.manylinux_2_\d+_x86_64)*", tag):
        return "linux-x86_64"
    if re.fullmatch(r"manylinux_2_28_aarch64(?:\.manylinux_2_\d+_aarch64)*", tag):
        return "linux-aarch64"
    if re.fullmatch(r"macosx_\d+_\d+_x86_64", tag):
        return "macos-x86_64"
    if re.fullmatch(r"macosx_\d+_\d+_arm64", tag):
        return "macos-arm64"
    if tag == "win_amd64":
        return "windows-amd64"
    raise ValueError(f"unqualified wheel platform: {tag}")


def check_python(directory, version):
    expected_version = python_version(version)
    wheels = sorted(directory.glob("*.whl"))
    require(len(wheels) == len(PLATFORMS), "exactly five native wheels are required")
    seen = set()
    inventory = []
    for path in wheels:
        match = re.fullmatch(r"infimount-([^-]+)-cp310-abi3-(.+)\.whl", path.name)
        require(match is not None, f"unexpected wheel name or ABI: {path.name}")
        require(match[1] == expected_version, f"wheel version mismatch: {path.name}")
        platform = wheel_platform(match[2])
        require(platform not in seen, f"duplicate wheel platform: {platform}")
        seen.add(platform)
        with zipfile.ZipFile(path) as archive:
            members = archive.infolist()
            require(all(safe_member(item.filename) for item in members), "unsafe wheel member")
            require(sum(item.file_size for item in members) <= 256 * 1024 * 1024, "wheel uncompressed size exceeds limit")
            metadata_names = [item.filename for item in members if item.filename.endswith(".dist-info/METADATA")]
            wheel_names = [item.filename for item in members if item.filename.endswith(".dist-info/WHEEL")]
            require(len(metadata_names) == len(wheel_names) == 1, "wheel metadata is ambiguous")
            require(archive.getinfo(metadata_names[0]).file_size <= 1024 * 1024, "wheel metadata exceeds limit")
            metadata = email.parser.BytesParser().parsebytes(archive.read(metadata_names[0]))
            require(metadata["Name"] == "infimount" and metadata["Version"] == expected_version, "wheel metadata disagrees with release")
            require(metadata["Requires-Python"] == ">=3.10", "wheel Python version contract mismatch")
            wheel_metadata = email.parser.BytesParser().parsebytes(archive.read(wheel_names[0]))
            tags = wheel_metadata.get_all("Tag") or []
            require(tags and all(tag.startswith("cp310-abi3-") for tag in tags), "wheel lacks the qualified ABI3 tag")
            require(set(tags) == {f"cp310-abi3-{tag}" for tag in match[2].split(".")}, "wheel filename and metadata platform disagree")
            require(any(item.filename.startswith("infimount/_native") and item.filename.endswith((".so", ".pyd")) for item in members), "wheel is missing the native extension")
            require("infimount/__init__.py" in archive.namelist(), "wheel is missing the public package")
        inventory.append({"file": path.name, "platform": platform, "sha256": file_hash(path), "bytes": path.stat().st_size})
    require(seen == PLATFORMS, "wheel platform coverage is incomplete")
    sources = sorted(directory.glob("infimount-*.tar.gz"))
    require(len(sources) == 1, "exactly one Python source archive is required")
    require(sources[0].name == f"infimount-{expected_version}.tar.gz", "source archive filename disagrees with release")
    with tarfile.open(sources[0], "r:gz") as archive:
        members = archive.getmembers()
        require(all(safe_member(item.name) and not (item.issym() or item.islnk()) for item in members), "unsafe source archive member")
        metadata_names = [item for item in members if item.name.endswith("/PKG-INFO")]
        require(len(metadata_names) == 1, "source archive metadata is ambiguous")
        require(metadata_names[0].size <= 1024 * 1024, "source metadata exceeds limit")
        metadata = email.parser.BytesParser().parsebytes(archive.extractfile(metadata_names[0]).read())
        require(metadata["Name"] == "infimount" and metadata["Version"] == expected_version, "source archive version disagrees with release")
        require(any(item.name.endswith("/Cargo.lock") for item in members), "source archive lacks the qualified dependency lock")
    source = sources[0]
    inventory.append({"file": source.name, "platform": "source", "sha256": file_hash(source), "bytes": source.stat().st_size})
    return inventory


def check_image(directory, tag, revision):
    manifest = json.loads((directory / "hosted-image.json").read_text())
    require(manifest.get("schema_version") == 1, "hosted image schema mismatch")
    require(manifest.get("tag") == tag, "hosted image tag mismatch")
    require(manifest.get("source_commit") == revision and re.fullmatch(r"[0-9a-f]{40}", revision), "hosted image source mismatch")
    require(manifest.get("image") == "ghcr.io/infimount/infimount-hosted", "hosted image repository mismatch")
    require(DIGEST.fullmatch(manifest.get("digest", "")), "invalid hosted image digest")
    platforms = manifest.get("platforms", [])
    require(len(platforms) == 2 and {row.get("platform") for row in platforms} == {"linux/amd64", "linux/arm64"}, "hosted image architecture coverage mismatch")
    for row in platforms:
        require(DIGEST.fullmatch(row.get("digest", "")) and DIGEST.fullmatch(row.get("config_digest", "")), "invalid platform digest")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("tag")
    parser.add_argument("--revision", default="")
    parser.add_argument("--python-only", action="store_true")
    parser.add_argument("--write-inventory", action="store_true")
    args = parser.parse_args()
    require(args.tag.startswith("v"), "release tag must start with v")
    files = check_python(args.directory, args.tag[1:])
    if not args.python_only:
        check_image(args.directory, args.tag, args.revision)
        if not args.write_inventory:
            inventory = json.loads((args.directory / "python-distribution.json").read_text())
            require(inventory == {"schema_version": 1, "tag": args.tag, "source_commit": args.revision, "files": files}, "published Python inventory disagrees with actual payloads")
    if args.write_inventory:
        require(re.fullmatch(r"[0-9a-f]{40}", args.revision), "inventory requires the exact source revision")
        (args.directory / "python-distribution.json").write_text(json.dumps({"schema_version": 1, "tag": args.tag, "source_commit": args.revision, "files": files}, indent=2) + "\n")
    print(f"Distribution check passed: five native ABI3 wheels and one isolated-build source archive ({args.tag})")


if __name__ == "__main__":
    main()
