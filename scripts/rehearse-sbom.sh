#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; OUT="${1:?output directory}"; VERSION="${2:?version}"
mkdir -p "$OUT/sbom-components"
for platform in linux macos windows; do printf '#!/bin/sh\necho infimount_mcp %s\n' "$VERSION" > "$OUT/sbom-components/infimount_mcp-$platform"; chmod +x "$OUT/sbom-components/infimount_mcp-$platform"; done
cat > "$OUT/SBOM.spdx.json" <<'JSON'
{"spdxVersion":"SPDX-2.3","SPDXID":"SPDXRef-DOCUMENT","name":"infimount-rehearsal","documentNamespace":"https://example.invalid/rehearsal","creationInfo":{"created":"1970-01-01T00:00:00Z","creators":["Tool: infimount-rehearsal"]},"packages":[],"files":[],"relationships":[]}
JSON
node "$ROOT_DIR/scripts/add-sidecar-to-sbom.mjs" "$OUT/SBOM.spdx.json" "$OUT/sbom-components" "$VERSION"
jq -e --arg v "$VERSION" '.packages[] | select(.name=="infimount_mcp") | .versionInfo==$v and (.hasFiles|length)==3' "$OUT/SBOM.spdx.json" >/dev/null
