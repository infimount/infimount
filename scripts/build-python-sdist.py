#!/usr/bin/env python3
"""Build a reproducible Python sdist with a qualified, minimal workspace lock.

Maturin packages only the Python crate and its path dependencies, but copies the
full workspace Cargo.lock. Prune that copied lock offline, reject changed
package identities/checksums and introduced edges, then require locked metadata.
This script never compiles Rust and never modifies the repository lockfile.
Requires Python 3.11 or later, Maturin 1.16.0 and the pinned Rust toolchain.
"""
from __future__ import annotations

import argparse
import email.parser
import gzip
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib

MAX_MEMBERS = 20_000
MAX_UNPACKED_BYTES = 256 * 1024 * 1024


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def extract_source(archive_path: Path, destination: Path) -> Path:
    """Extract only bounded regular files/directories under one safe top level."""
    seen: set[str] = set()
    roots: set[str] = set()
    total = 0
    with tarfile.open(archive_path, "r:gz") as archive:
        for index, member in enumerate(archive):
            require(index < MAX_MEMBERS, "source archive contains too many entries")
            name = member.name.rstrip("/") if member.isdir() else member.name
            path = PurePosixPath(name)
            require(
                bool(name) and bool(path.parts) and not path.is_absolute() and ".." not in path.parts
                and "\\" not in name and ":" not in name and str(path) == name
                and not any(ord(character) < 32 for character in name),
                "source archive contains an unsafe path",
            )
            require(member.isdir() or member.isreg(), "source archive contains a link or special file")
            require(name not in seen, "source archive contains duplicate paths")
            require(member.size >= 0, "source archive contains a negative file size")
            seen.add(name)
            roots.add(path.parts[0])
            require(len(roots) == 1, "source archive has multiple top-level entries")
            total += member.size
            require(total <= MAX_UNPACKED_BYTES, "source archive exceeds the unpacked size limit")
            target = destination.joinpath(*path.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                target.chmod(0o755)
                continue
            require(len(path.parts) > 1, "source files must be below a top-level directory")
            target.parent.mkdir(parents=True, exist_ok=True)
            stream = archive.extractfile(member)
            require(stream is not None, "source archive file has no payload")
            with stream, target.open("xb") as output:
                shutil.copyfileobj(stream, output, length=1024 * 1024)
            require(target.stat().st_size == member.size, "source archive file was truncated")
            target.chmod(0o755 if member.mode & 0o111 else 0o644)
    require(bool(roots), "source archive is empty")
    root = destination / roots.pop()
    require(root.is_dir(), "source archive root is not a directory")
    return root


def package_map(lock: dict) -> dict[tuple[str, str, str | None], dict]:
    packages = {}
    for package in lock.get("package", []):
        key = (package["name"], package["version"], package.get("source"))
        require(key not in packages, "dependency lock contains duplicate package identities")
        packages[key] = package
    require(bool(packages), "dependency lock contains no packages")
    return packages


def canonical_edges(package: dict, packages: dict) -> set[tuple[str, str, str | None]]:
    """Resolve Cargo's shortened edge strings against this lock's package set."""
    edges = set()
    for edge in package.get("dependencies", []):
        require(isinstance(edge, str), "dependency lock contains a non-string edge")
        # Cargo serializes `name`, `name version`, or `name version (source)`.
        # A source qualifier on a Git edge can omit the precise commit fragment,
        # while the canonical package identity always retains that locked commit.
        parts = edge.split(" ", 2)
        name = parts[0]
        version = parts[1] if len(parts) >= 2 else None
        require(
            bool(name) and not any(character.isspace() for character in name)
            and (version is None or bool(version) and not any(character.isspace() for character in version)),
            "dependency lock contains a malformed edge",
        )
        source = None
        if len(parts) == 3:
            require(parts[2].startswith("(") and parts[2].endswith(")"), "dependency edge has a malformed source qualifier")
            source = parts[2][1:-1]
            require(bool(source) and not any(character.isspace() for character in source), "dependency edge has an empty or malformed source qualifier")
        matches = []
        for identity in packages:
            if identity[0] != name or version is not None and identity[1] != version:
                continue
            locked_source = identity[2]
            source_matches = source is None or source == locked_source or (
                locked_source is not None and locked_source.startswith("git+")
                and "#" not in source and source == locked_source.split("#", 1)[0]
            )
            if source_matches:
                matches.append(identity)
        require(len(matches) == 1, "dependency edge is unresolved or ambiguous in its own lock")
        require(matches[0] not in edges, "dependency lock contains duplicate edge identities")
        edges.add(matches[0])
    return edges


def verify_pruned_lock(qualified: dict, candidate: dict) -> dict[str, int]:
    """Allow removals and equivalent edge spelling, never package/source changes."""
    require(qualified.get("version") == candidate.get("version"), "dependency lock format changed")
    old = package_map(qualified)
    new = package_map(candidate)
    retained_edges = 0
    for key, package in new.items():
        require(key in old, "packaged lock introduced an unqualified package version or source")
        # Cargo may shorten dependency-edge spellings after unused duplicate
        # versions disappear. All identity, checksum and replacement fields must
        # remain exactly those from the already-qualified root lock.
        retained = {field: value for field, value in package.items() if field != "dependencies"}
        original = {field: value for field, value in old[key].items() if field != "dependencies"}
        require(retained == original, "packaged lock changed qualified package metadata or checksum")
        original_edges = canonical_edges(old[key], old)
        candidate_edges = canonical_edges(package, new)
        require(candidate_edges <= original_edges, "packaged lock introduced an unqualified dependency edge")
        retained_edges += len(candidate_edges)
    return {
        "qualified_packages": len(old),
        "packaged_packages": len(new),
        "removed_packages": len(old) - len(new),
        "unchanged_external_packages": sum(key[2] is not None for key in new),
        "unchanged_dependency_edges": retained_edges,
    }


def pack_source(root: Path, destination: Path, epoch: int) -> None:
    """Sort entries and normalize tar/gzip timestamps, ownership and permissions."""
    require(0 <= epoch <= 0xFFFFFFFF, "SOURCE_DATE_EPOCH is outside the gzip timestamp range")
    paths = [root, *sorted(root.rglob("*"), key=lambda path: path.relative_to(root).as_posix())]
    with destination.open("wb") as output:
        with gzip.GzipFile(filename="", mode="wb", fileobj=output, mtime=epoch, compresslevel=9) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for path in paths:
                    require(not path.is_symlink(), "packaging source acquired a symbolic link")
                    require(path.is_dir() or path.is_file(), "packaging source acquired a special file")
                    name = PurePosixPath(root.name) / PurePosixPath(path.relative_to(root).as_posix())
                    info = tarfile.TarInfo(str(name))
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    info.mtime = epoch
                    info.mode = 0o755 if path.is_dir() or path.stat().st_mode & 0o111 else 0o644
                    if path.is_dir():
                        info.type = tarfile.DIRTYPE
                        archive.addfile(info)
                    else:
                        info.size = path.stat().st_size
                        with path.open("rb") as stream:
                            archive.addfile(info, stream)


def run(command: list[str], directory: Path) -> None:
    result = subprocess.run(command, cwd=directory, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    if result.returncode:
        raise RuntimeError(f"Packaging command failed: {command[0]}\n{result.stderr[-8000:]}")


def source_epoch(repository: Path) -> int:
    if "SOURCE_DATE_EPOCH" in os.environ:
        return int(os.environ["SOURCE_DATE_EPOCH"])
    result = subprocess.run(
        ["git", "log", "-1", "--format=%ct"], cwd=repository,
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
    )
    return int(result.stdout.strip()) if result.returncode == 0 and result.stdout.strip() else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, help="output directory, defaults to python/dist")
    args = parser.parse_args()
    repository = Path(__file__).resolve().parent.parent
    output_directory = (args.out or repository / "python/dist").resolve()
    require(importlib.metadata.version("maturin") == "1.16.0", "packaging requires Maturin 1.16.0")
    license_bytes = (repository / "LICENSE").read_bytes()
    require((repository / "python/LICENSE").read_bytes() == license_bytes, "Python LICENSE differs from the repository LICENSE")
    root_lock_path = repository / "Cargo.lock"
    root_lock_bytes = root_lock_path.read_bytes()
    qualified = tomllib.loads(root_lock_bytes.decode("utf-8"))
    epoch = source_epoch(repository)

    with tempfile.TemporaryDirectory(prefix="infimount-python-sdist-") as temporary:
        temporary_root = Path(temporary)
        raw_directory = temporary_root / "raw"
        raw_directory.mkdir()
        run(
            [sys.executable, "-m", "maturin", "sdist", "--out", str(raw_directory)],
            repository / "python",
        )
        archives = list(raw_directory.glob("infimount-*.tar.gz"))
        require(len(archives) == 1, "Maturin must produce exactly one Infimount source archive")
        unpack_directory = temporary_root / "unpacked"
        unpack_directory.mkdir()
        packaged = extract_source(archives[0], unpack_directory)
        packaged_lock = packaged / "Cargo.lock"
        require(packaged_lock.read_bytes() == root_lock_bytes, "Maturin did not copy the qualified root dependency lock")
        metadata = email.parser.BytesParser().parsebytes((packaged / "PKG-INFO").read_bytes())
        require(metadata["License-Expression"] == "MIT", "source metadata must declare the MIT license")
        require("LICENSE" in (metadata.get_all("License-File") or []), "source metadata must include LICENSE")
        require((packaged / "LICENSE").read_bytes() == license_bytes, "source archive LICENSE differs from the repository LICENSE")
        files_before = {path.relative_to(packaged) for path in packaged.rglob("*") if path.is_file()}

        # --workspace updates the remaining path packages while retaining their
        # locked external dependencies. Offline resolution cannot select remote
        # updates. The comparison below additionally rejects any cached change.
        run(["cargo", "+1.94.0", "update", "--workspace", "--offline"], packaged)
        candidate_bytes = packaged_lock.read_bytes()
        counts = verify_pruned_lock(qualified, tomllib.loads(candidate_bytes.decode("utf-8")))
        run(["cargo", "+1.94.0", "metadata", "--locked", "--offline", "--format-version", "1"], packaged)
        require(packaged_lock.read_bytes() == candidate_bytes, "locked metadata changed the packaged lock")
        files_after = {path.relative_to(packaged) for path in packaged.rglob("*") if path.is_file()}
        require(files_after == files_before, "dependency resolution unexpectedly changed the packaged file inventory")
        require(root_lock_path.read_bytes() == root_lock_bytes, "packaging changed the repository dependency lock")
        output_directory.mkdir(parents=True, exist_ok=True)
        output = output_directory / archives[0].name
        staged_output = temporary_root / archives[0].name
        pack_source(packaged, staged_output, epoch)
        # os.replace is atomic only within a filesystem. Copy a verified archive
        # to a temporary sibling, then publish that sibling with one rename.
        with tempfile.NamedTemporaryFile(prefix=".infimount-sdist-", dir=output_directory, delete=False) as staging:
            staging_path = Path(staging.name)
        try:
            shutil.copyfile(staged_output, staging_path)
            os.replace(staging_path, output)
        finally:
            staging_path.unlink(missing_ok=True)
        report = {
            "archive": str(output),
            "sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
            "qualified_lock_sha256": hashlib.sha256(root_lock_bytes).hexdigest(),
            "packaged_lock_sha256": hashlib.sha256(candidate_bytes).hexdigest(),
            "locked_offline_metadata": "passed",
            **counts,
        }
        print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
