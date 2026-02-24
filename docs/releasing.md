# Releasing Infimount

This document is the operational checklist for cutting a release.

## 1. Pre-release checks

1. Ensure `main` is green in all required workflows.
2. If you want signed installers, configure release secrets first.
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
3. Confirm version is synced in:
   - `apps/desktop/package.json`
   - `apps/desktop/src-tauri/Cargo.toml`
   - `apps/desktop/src-tauri/tauri.conf.json`
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

- validate tag/version consistency
- build Linux, macOS, Windows binaries
- sign/notarize macOS artifacts if Apple signing secrets are present
- sign Windows installers if Windows signing secrets are present
- run artifact smoke checks
- generate SHA256 checksum files
- generate `SBOM.spdx.json`
- create GitHub release draft with all assets
- emit artifact provenance attestation

## 3. Validate draft release

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
2. Verify checksums:
   - download at least one binary and `SHA256SUMS.txt`
   - run `sha256sum -c SHA256SUMS.txt`
3. Perform quick install sanity on each target OS where possible.
4. Publish release.

## 4. Post-release checks

1. Confirm `/releases/latest/download/...` links resolve.
2. Confirm GitHub Pages download page still works.
3. Confirm release notes render as expected.

## 5. Rollback strategy

If a bad release is published:

1. Mark the release as pre-release or draft again.
2. Delete incorrect assets from the release page.
3. Push a fix and tag a new patch release (`vX.Y.(Z+1)`).
4. Do not reuse/retag an existing published version.
