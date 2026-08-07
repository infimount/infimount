import fs from "node:fs";
import { canonicalVersionFromTag } from "./release-version-utils.mjs";

const tag = process.env.GITHUB_REF_NAME ?? "";
const version = canonicalVersionFromTag(tag);

const updateJsonVersion = (path) => {
  const value = JSON.parse(fs.readFileSync(path, "utf8"));
  value.version = version;
  fs.writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
};

updateJsonVersion("package.json");
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

console.log(`Release version synchronized from tag ${tag} -> ${version}`);
