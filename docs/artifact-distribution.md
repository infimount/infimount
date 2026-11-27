# Artifact service and Python distribution

Release status: this pipeline is part of the
[v0.8.2-rc.1 source candidate](release-notes-0.8.2-rc.1.md), prepared for automated
qualification. Candidate artifacts become available after release publication.
v0.8.1 remains the stable release.

The [canonical release workflow](../.github/workflows/release.yml) builds the
desktop application, hosted container and Python package from the same release
tag. The hosted and Python tracks are additional mandatory gates before the
release publisher runs. No separate task publishes unqualified builds.

## Qualification matrix

| Artifact | Native build and execution target | Validation |
| --- | --- | --- |
| Hosted image | Linux amd64 | Non-root, read-only root, no-network configuration check |
| Hosted image | Linux arm64 | The same check on a native arm64 runner |
| Python ABI3 wheel | Linux x86_64, glibc 2.28+ | Install and policy/binary tests on CPython 3.10 and 3.14 |
| Python ABI3 wheel | Linux aarch64, glibc 2.28+ | The same native installation tests |
| Python ABI3 wheel | macOS Intel | The same native installation tests |
| Python ABI3 wheel | macOS Apple silicon | The same native installation tests |
| Python ABI3 wheel | Windows amd64 | The same native installation tests |
| Python source archive | Linux x86_64 | Rebuild outside the checkout and test the installed result |

The wheel uses the standard CPython stable ABI beginning with 3.10. Free-threaded
CPython, PyPy and Windows arm64 are not qualified distribution targets. The Linux
wheels use the manylinux 2.28 baseline. Hosted local filesystem confinement is a
Linux service feature and is distinct from the Python client's trusted local-root
contract described in [Python bindings](python-bindings.md).

Pull requests run both [hosted qualification](../.github/workflows/hosted-data-plane.yml)
and [Python qualification](../.github/workflows/python-bindings.yml). Neither has
registry-write permission or a PyPI publication step. Release runs reuse those
workflows. The publisher loads the exact container archives that passed native
tests instead of rebuilding them in the publishing job.

## Container identity and access

The image repository is `ghcr.io/infimount/infimount-hosted`. Each release has its
own complete version tag, including `v` and any prerelease suffix. A candidate
such as `v0.8.2-rc.1` cannot move a stable tag. There is no floating `latest` alias.
The publisher refuses to replace an existing version with different platform
digests. Image staging uses run-specific tags. The final version tag is promoted
only after artifact checks, provenance and draft re-download validation pass.
GitHub package access policy determines whether registry authentication
is required. The repository's release token publishes and validates the package.

Download `hosted-image.json` and `SHA256SUMS.txt` from the desired release. Verify
the checksum file against its release provenance, then verify its payload checksums:

```bash
gh attestation verify SHA256SUMS.txt --repo infimount/infimount
sha256sum --check SHA256SUMS.txt
image="$(jq -r .image hosted-image.json)"
digest="$(jq -r .digest hosted-image.json)"
gh attestation verify "oci://$image@$digest" --repo infimount/infimount
docker pull "$image@$digest"
```

`SHA256SUMS.txt` covers the complete release payload. Download all listed files
before running the full checksum command. The image JSON records the index
digest, each native platform manifest digest, each image configuration digest
and the source commit. The image labels carry the same source revision and tag
version. The combined digest is the preferred deployment reference.

Image publication creates no service, network endpoint, identity provider or
cluster resources. [Deployment configuration](hosted-artifact-service.md) remains
explicit. Runtime secrets are never built into the image.

## Python installation and integrity

The release contains five `.whl` files, one source `.tar.gz` and
`python-distribution.json`. The JSON inventory records the source commit and each
file's SHA-256 hash, size and platform. All files are covered by the common release
checksums and GitHub build provenance. Python publication is through GitHub
Release assets. There is no assumed PyPI project, account or upload credential.

After verification, install the wheel for the machine and Python interpreter:

```bash
python -m pip install --no-index --find-links . infimount
```

Run this inside the verified release-download directory. Pip selects the
compatible local wheel and rejects incompatible platforms. The wheel filename
includes the version, `cp310-abi3` and platform tag. See the [binding guide](python-bindings.md) for complete
client configuration and governed operations.

## Publication checks

Before publication, the pipeline verifies the wheel names, metadata versions,
ABI, all five platforms, native extension presence and source archive. It checks
image architecture, non-root user, source labels, native image configuration
digests and combined manifest membership. Release assets include the inventories
before checksums and provenance are generated.

The publisher re-downloads and validates both draft and published release assets.
Post Release Validation repeats the inventory checks, verifies container
provenance, checks both platform digests and executes the downloaded amd64 image
by its immutable combined digest. Native arm64 execution is already a required
prepublication gate. A passing build is not evidence of a deployed production
service or a paid live-provider run.
