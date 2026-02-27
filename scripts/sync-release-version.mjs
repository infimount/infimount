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

const cargoTomlPath = "apps/desktop/src-tauri/Cargo.toml";
const cargoToml = fs.readFileSync(cargoTomlPath, "utf8");
const cargoVersionPattern = /^version\s*=\s*"[^"]*"/m;
if (!cargoVersionPattern.test(cargoToml)) {
  throw new Error("Could not locate top-level version field in apps/desktop/src-tauri/Cargo.toml");
}
const updatedCargoToml = cargoToml.replace(
  cargoVersionPattern,
  `version = "${version}"`,
);

fs.writeFileSync(cargoTomlPath, updatedCargoToml, "utf8");

console.log(
  `Release version synchronized from tag ${tag} -> ${version}${msiSafe ? " (MSI-safe mode)" : ""}`,
);
