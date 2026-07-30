#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const [sbomPath, sidecarDir, version] = process.argv.slice(2);
if (!sbomPath || !sidecarDir || !version) {
  throw new Error("usage: add-sidecar-to-sbom.mjs <SBOM.spdx.json> <sidecar-dir> <version>");
}

const sbom = JSON.parse(fs.readFileSync(sbomPath, "utf8"));
if (sbom.spdxVersion !== "SPDX-2.3" || !sbom.SPDXID) {
  throw new Error("SBOM is not an SPDX 2.3 document");
}

const sidecars = fs.readdirSync(sidecarDir)
  .filter((name) => /^infimount_mcp/.test(name))
  .sort();
if (sidecars.length < 3) {
  throw new Error(`expected at least three platform sidecars, found ${sidecars.length}`);
}

const packageId = "SPDXRef-Package-infimount-mcp";
const files = sidecars.map((name, index) => {
  const bytes = fs.readFileSync(path.join(sidecarDir, name));
  return {
    fileName: `sidecars/${name}`,
    SPDXID: `SPDXRef-File-infimount-mcp-${index + 1}`,
    checksums: [{ algorithm: "SHA256", checksumValue: crypto.createHash("sha256").update(bytes).digest("hex") }],
    licenseConcluded: "NOASSERTION",
    copyrightText: "NOASSERTION",
  };
});

sbom.packages = (sbom.packages ?? []).filter((item) => item.SPDXID !== packageId);
sbom.files = (sbom.files ?? []).filter((item) => !String(item.SPDXID).startsWith("SPDXRef-File-infimount-mcp-"));
sbom.relationships = (sbom.relationships ?? []).filter((item) =>
  item.spdxElementId !== packageId
  && !String(item.relatedSpdxElement).startsWith("SPDXRef-File-infimount-mcp-"),
);

sbom.packages.push({
  name: "infimount_mcp",
  SPDXID: packageId,
  versionInfo: version,
  downloadLocation: "NOASSERTION",
  filesAnalyzed: true,
  licenseConcluded: "NOASSERTION",
  licenseDeclared: "NOASSERTION",
  copyrightText: "NOASSERTION",
  supplier: "Organization: Infimount",
  primaryPackagePurpose: "APPLICATION",
  hasFiles: files.map((file) => file.SPDXID),
});
sbom.files.push(...files);
sbom.relationships.push({
  spdxElementId: sbom.SPDXID,
  relationshipType: "DESCRIBES",
  relatedSpdxElement: packageId,
});
for (const file of files) {
  sbom.relationships.push({
    spdxElementId: packageId,
    relationshipType: "CONTAINS",
    relatedSpdxElement: file.SPDXID,
  });
}

fs.writeFileSync(sbomPath, `${JSON.stringify(sbom, null, 2)}\n`, "utf8");
console.log(`Added infimount_mcp ${version} (${files.length} platform binaries) to ${sbomPath}`);
