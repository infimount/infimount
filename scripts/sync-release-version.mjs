import fs from "node:fs";

const tag = process.env.GITHUB_REF_NAME ?? "";
const version = tag.replace(/^v/, "");

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

console.log(`Release version synchronized from tag ${tag} -> ${version}`);
