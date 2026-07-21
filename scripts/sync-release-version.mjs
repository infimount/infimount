import fs from "node:fs";

const tag = process.env.GITHUB_REF_NAME ?? "";
const rawVersion = tag.replace(/^v/, "");
const msiSafe = process.argv.includes("--msi-safe");

const toMsiSafeVersion = (version) => {
  const [core, prerelease] = version.split("-", 2);
  if (!prerelease) return version;

  const parts = prerelease.split(".");
  const numericPart = [...parts].reverse().find((part) => /^[0-9]+$/.test(part));
  const msiPrerelease = numericPart ?? "0";
  const msiPrereleaseNumber = Number.parseInt(msiPrerelease, 10);

  if (Number.isNaN(msiPrereleaseNumber) || msiPrereleaseNumber < 0 || msiPrereleaseNumber > 65535) {
    throw new Error(
      `MSI prerelease identifier must be numeric and <= 65535 (got: ${msiPrerelease})`,
    );
  }

  return `${core}-${msiPrerelease}`;
};

const version = msiSafe ? toMsiSafeVersion(rawVersion) : rawVersion;

if (!version) {
  throw new Error("Could not derive release version from GITHUB_REF_NAME");
}

const updateJsonVersion = (path) => {
  const value = JSON.parse(fs.readFileSync(path, "utf8"));
  value.version = version;
  fs.writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
};

updateJsonVersion("apps/desktop/package.json");
updateJsonVersion("apps/desktop/src-tauri/tauri.conf.json");

const workspaceCargoPath = "Cargo.toml";
const workspaceCargo = fs.readFileSync(workspaceCargoPath, "utf8");
const workspaceVersionPattern = /^version\s*=\s*"[^"]*"/m;
if (!workspaceVersionPattern.test(workspaceCargo)) {
  throw new Error("Could not locate workspace version field in Cargo.toml");
}
const updatedWorkspaceCargo = workspaceCargo.replace(
  workspaceVersionPattern,
  `version = "${version}"`,
);
fs.writeFileSync(workspaceCargoPath, updatedWorkspaceCargo, "utf8");

console.log(
  `Release version synchronized from tag ${tag} -> ${version}${msiSafe ? " (MSI-safe mode)" : ""}`,
);
