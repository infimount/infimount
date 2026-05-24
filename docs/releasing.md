# Releasing Infimount

This document is the operational checklist for cutting a release.

## 1. Pre-release checks

### Zero manual product test execution

Infimount releases are intended to require **zero manual product test execution**. Manual product test execution must not be a release gate. Before a tag can produce release artifacts, the `Release` workflow runs automated release-gate jobs for frontend tests, Playwright UI tests, Rust tests/coverage, desktop smoke, OpenDAL storage simulator verification, and a release-policy guard that verifies artifact build jobs still depend on those gates.

Optional local dry run before tagging:

```bash
pnpm test:release
```

For faster local iteration only, you may skip slow checks with:

```bash
INFIMOUNT_RELEASE_GATE_SKIP_UI=1 \
INFIMOUNT_RELEASE_GATE_SKIP_DESKTOP_SMOKE=1 \
INFIMOUNT_RELEASE_GATE_SKIP_RUST_COVERAGE=1 \
INFIMOUNT_RELEASE_GATE_SKIP_STORAGE_SIMULATOR=1 \
pnpm test:release
```

Required release preparation:

1. Ensure `main` is green in all required workflows.
2. Choose next version by impact:
   - patch (`0.1.1`) for fixes only
   - minor (`0.2.0`) for new user-facing features (recommended for MCP rollout)
3. If you want signed installers, configure release secrets first.
   - macOS signing/notarization secrets:
     - `APPLE_CERTIFICATE`
     - `APPLE_CERTIFICATE_PASSWORD`
     - `APPLE_SIGNING_IDENTITY`
     - `APPLE_ID`
     - `APPLE_PASSWORD`
     - `APPLE_TEAM_ID`
   - Windows signing secrets:
     - `WINDOWS_CERTIFICATE_BASE64`
     - `WINDOWS_CERTIFICATE_PASSWORD`
4. Update `CHANGELOG.md`.
5. Confirm no secrets or local artifacts are staged:
   - `git status`
   - `git grep -nE "(AKIA|BEGIN PRIVATE KEY|AIza|SECRET|TOKEN)"` (quick heuristic)

## 2. Create and push tag

```bash
git checkout main
git pull --ff-only
git tag vX.Y.Z
git push origin vX.Y.Z
```

The `Release` workflow is triggered by `v*` tags and will:

- block release builds until automated release gates pass:
  - frontend lint, typecheck, unit tests, integration tests, coverage, and production build
  - Playwright component/UI tests
  - Rust format, clippy, workspace tests, and coverage gate
  - desktop launch/migration smoke test under Xvfb
  - OpenDAL storage simulator verification, including read/write/list/stat/delete round trips where supported and WebDAV list reachability
  - zero-manual release policy check (`scripts/check-zero-manual-release-gate.sh`)
- sync app manifest versions from the pushed tag via `scripts/sync-release-version.mjs`
- build Linux, macOS, Windows binaries
- sign/notarize macOS artifacts if Apple signing secrets are present
- sign Windows installers if Windows signing secrets are present
- run artifact smoke checks, including Linux AppImage launch/migration, `.deb` install/launch/migration, and RPM package metadata checks
- generate SHA256 checksum files
- generate `SBOM.spdx.json`
- create GitHub release draft with all assets
- emit artifact provenance attestation

## 3. Validate draft release

The release workflow already performs automated artifact presence, checksum, updater metadata, package, and provenance checks. The remaining human action is release approval/publishing, not manual product testing.

In the release draft:

1. Confirm all expected assets exist:
   - `Infimount-amd64.deb`
   - `Infimount-x86_64.rpm`
   - `Infimount-x86_64.AppImage`
   - `Infimount.dmg`
   - `Infimount.msi`
   - `Infimount-setup.exe`
   - `SHA256SUMS.txt`
   - `*.sha256`
   - `SBOM.spdx.json`
2. Confirm the generated release notes are acceptable.
3. Publish release.

Manual checksum or install sanity checks are optional spot audits only; they are no longer required release tests because the workflow validates checksums, package structure, and Linux artifact launch/install smoke paths automatically.

## 4. Post-release checks

1. Confirm `/releases/latest/download/...` links resolve.
2. Confirm GitHub Pages download page still works.
3. Confirm release notes render as expected.
4. Update Homebrew tap repo (`infimount/homebrew-infimount`):
   - bump Formula and Cask to the released tag
   - update checksums from release assets
   - validate locally:
     - `brew tap infimount/infimount`
     - `brew install infimount`
     - `brew install --cask infimount` (macOS)
5. Merge/publish Homebrew tap changes.
6. Merge the automated version-sync PR (workflow: `Sync Version After Release`) so `main` app manifests reflect the published tag.

## 5. Rollback strategy

If a bad release is published:

1. Mark the release as pre-release or draft again.
2. Delete incorrect assets from the release page.
3. Push a fix and tag a new patch release (`vX.Y.(Z+1)`).
4. Do not reuse/retag an existing published version.
