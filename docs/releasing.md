# Releasing Infimount

This document is the operational checklist for cutting a release.

## 1. Pre-release checks

### Zero manual product test execution

Infimount releases are intended to require **zero manual product test execution**. Manual product test execution must not be a release gate. Before a tag can produce release artifacts, the `Release` workflow runs automated release-gate jobs for frontend tests, Playwright UI tests, Rust tests/coverage, desktop smoke, OpenDAL storage simulator verification, release consistency, feature-doc consistency, install-script smoke, and a release-policy guard that verifies artifact build jobs still depend on those gates.

For UI work, every newly added or changed visible action must be covered by an automated test that performs the action, not just asserts that the control renders. At least one Playwright component/UI test should capture a screenshot snapshot of the intended post-action state for each changed screen-level flow. The split-pane regression test is the reference pattern: open the visible action, assert the resulting controls/copy, assert removed controls stay absent, close the mode, and keep the screenshot under `apps/desktop/playwright/__snapshots__/`.

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
   - patch (`X.Y.Z+1`) for fixes only
   - minor (`X.Y+1.0`) for new user-facing features
3. Configure the updater signing secrets before every release tag. Every release fails before builds if updater signing material is absent. Apple and Windows platform-signing material is optional; when fully configured, platform signatures are built and verified, and when absent, the release is explicitly marked platform-unsigned. Partial platform configuration fails closed.
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
   - Tauri updater signing secrets:
     - `TAURI_SIGNING_PRIVATE_KEY`
     - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
4. Prepare public release identity:
   - For a prerelease such as `v0.8.0-rc.1`, add `docs/release-notes-0.8.0-rc.1.md`; README, Pages, and llms continue to identify the previous stable release.
   - Before a stable tag, run `node scripts/prepare-stable-release-docs.mjs vX.Y.Z`, review and commit its README, CHANGELOG, Pages, and llms changes, then tag that prepared commit. Stable consistency validation rejects `not published yet` language.
   - Release jobs derive manifest versions from the exact tag before running consistency checks. SemVer `+build` metadata is rejected.
5. Confirm no secrets or local artifacts are staged:
   - `git status`
   - `git grep -nE "(AKIA|BEGIN PRIVATE KEY|AIza|SECRET|TOKEN)"` (quick heuristic)

## 2. Hermetic release rehearsal

Before requesting a signed prerelease, run the non-publishing rehearsal:

```bash
pnpm test:release:rehearsal
# retain evidence for inspection:
bash scripts/release-rehearsal.sh --version 0.8.0-rc.1 --work-dir /tmp/infimount-rehearsal --keep
```

This uses only temporary updater keys and deterministic local fixtures. It validates updater signatures and tamper rejection, exact-tag metadata, checksums, SBOM sidecar coverage, fake upload/download byte round-trips, package-sidecar extraction, fixture secret scans, and the prerelease/stable signing-policy matrix. It never invokes `gh`, publishes a release, contacts GitHub, or consumes production secrets. The `Release Rehearsal` workflow runs the Linux aggregate and native macOS/Windows tool checks with read-only permissions.

The rehearsal proves signing **integrity** only. It does not prove Apple notarization/stapling or Gatekeeper trust, trusted Windows Authenticode/SmartScreen reputation, production updater-key correspondence, GitHub publication permissions, or real provider behavior. Those external checks remain required for the production prerelease/stable workflow.

## 3. Create and push tag

```bash
git checkout main
git pull --ff-only
git tag vX.Y.Z
git push origin vX.Y.Z
```

The `Release` workflow is triggered by `v*` tags and will:

- block release builds until automated release gates pass:
  - frontend lint, typecheck, unit tests, integration tests, coverage, and production build
  - Playwright component/UI tests, including screenshot snapshots for changed screen-level visible-action flows
  - Rust format, clippy, workspace tests, and coverage gate
  - desktop launch/migration smoke test under Xvfb
  - OpenDAL storage simulator verification, including read/write/list/stat/delete round trips where supported and WebDAV list reachability
  - optional credential-gated OAuth storage smoke via `scripts/oauth-storage-smoke.sh` when Google Drive or OneDrive test credentials are intentionally supplied; this is not a zero-manual release gate because provider OAuth APIs have no local emulator
  - release consistency checks for app versions, README, GitHub Pages, `CHANGELOG.md`, `docs/llms.txt`, and `docs/release-notes-X.Y.Z.md`
  - feature-doc consistency checks for supported backend names, S3-compatible wording, representative MCP tool names, Workbench copy, and Agent Workspaces copy
  - install-script checksum smoke tests for Linux/macOS shell and Windows PowerShell installers
  - zero-manual release policy check (`scripts/check-zero-manual-release-gate.sh`)
- sync app manifest versions from the pushed tag via `scripts/sync-release-version.mjs` before consistency validation and every platform build
- build Linux, macOS, Windows binaries
- require updater signing and private/public key correspondence for every release tag; platform signing is used and verified when credentials are configured, otherwise stable and prerelease packages are explicitly marked platform-unsigned
- preserve mandatory checksums, SBOM, provenance, artifact verification, and package-bound sidecar integrity; platform-unsigned applications must never be presented as notarized or Authenticode-signed
- run artifact smoke checks, including Linux AppImage launch/migration, `.deb` install/launch/migration, and sidecar extraction/version checks for AppImage, DEB, RPM, DMG, MSI, and NSIS installers
- validate release asset presence, updater metadata, checksum entries, and per-file `.sha256` files
- generate `SHA256SUMS.txt` and per-file checksum files for every published payload, including updater archives, updater signatures, metadata, installers, scripts, and SBOM
- generate mandatory `SBOM.spdx.json` from the release assets plus the three platform sidecars, then require an explicit `infimount_mcp` component with platform-binary checksums
- validate all collected assets, checksums, cryptographic updater signatures, updater URL references, install-script fixtures, SBOM sidecar coverage, and provenance inputs before and after draft upload
- create a draft release, re-download and validate the uploaded assets and checksums, then automatically publish stable tags only after those validations pass; prerelease tags publish with prerelease status
- emit artifact provenance attestation

## 3. Validate the published release

The release workflow performs automated artifact presence, checksum, updater metadata, install-script, package, and provenance checks before publication. After publication, confirm the expected assets exist:

1. Expected assets:
   - `Infimount-amd64.deb`
   - `Infimount-x86_64.rpm`
   - `Infimount-x86_64.AppImage`
   - `Infimount.dmg`
   - `Infimount.msi`
   - `Infimount-setup.exe`
   - `install.sh`
   - `install.ps1`
   - `SHA256SUMS.txt`
   - updater payload archive(s) and matching `.sig` files referenced by `latest.json`
   - `*.sha256` for every published payload
   - `SBOM.spdx.json` with an `infimount_mcp` component
2. Confirm the generated release notes and stable/prerelease marker are correct.

Manual checksum or install sanity checks are optional spot audits only; they are no longer required release tests because the workflow validates checksums, package structure, and Linux artifact launch/install smoke paths automatically.

### Optional signing verification

The release workflow imports the Windows certificate before bundling, signs the sidecar first, lets Tauri sign the app/installers/updater chain, and verifies Authenticode on the app executable, sidecar, installers, extracted installer executables, and executable updater payloads. It also runs macOS `codesign`, Gatekeeper, sidecar-signature, and notarization-ticket checks before publication. Users can independently spot-check downloaded artifacts:

```bash
# macOS
codesign --verify --deep --strict --verbose=2 /Applications/Infimount.app
spctl --assess --type execute --verbose=2 /Applications/Infimount.app
xcrun stapler validate Infimount.dmg

# Windows (Developer Command Prompt)
signtool verify /pa /all /v Infimount.msi
signtool verify /pa /all /v Infimount-setup.exe
```

The updater public key is embedded in `apps/desktop/src-tauri/tauri.conf.json`; updater signatures are produced only from the corresponding protected private key. Platform sidecar copies are used only to produce and validate the SBOM and are not published as standalone downloads. Installed sidecars live inside each platform's application resources (for example, `Infimount.app/Contents/MacOS/mcp` on macOS and the Infimount installation directory on Windows/Linux), not on the user's `PATH`.

## 4. Post-release checks

The `Post Release Validation` workflow runs automatically when a release is published. It verifies tag-specific release links, release/docs consistency, and Homebrew checksum resolution. If `HOMEBREW_TAP_DISPATCH_TOKEN` is configured, it dispatches the Homebrew tap update workflow.

Manual spot checks remain optional:

1. Confirm GitHub Pages download page still works.
2. Confirm release notes render as expected.
3. For stable releases only, update the Homebrew tap repo (`infimount/homebrew-infimount`) if the dispatch token is not configured. Prereleases never dispatch or update Homebrew:
   - bump Formula and Cask to the released tag
   - update checksums from release assets
   - validate locally:
     - `brew tap infimount/infimount`
     - `brew install infimount`
     - `brew install --cask infimount` (macOS)
4. Merge/publish Homebrew tap changes when not handled by automation.
5. Merge the automated stable-release identity PR (workflow: `Sync Version After Release`) if the stable tag was not already cut from an identity-prepared commit. The workflow is intentionally skipped for prereleases.

## 5. Rollback strategy

If a bad release is published:

1. Mark the release as pre-release or draft again.
2. Delete incorrect assets from the release page.
3. Push a fix and tag a new patch release (`vX.Y.(Z+1)`).
4. Do not reuse/retag an existing published version.
