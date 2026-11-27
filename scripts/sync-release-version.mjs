import fs from "node:fs";
import { canonicalVersionFromTag } from "./release-version-utils.mjs";

const tag = process.argv[2] || process.env.GITHUB_REF_NAME || "";
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

// Keep locked release builds on the already-qualified dependency graph. Only
// workspace package versions change when the tag is applied.
const lockPath = "Cargo.lock";
if (fs.existsSync(lockPath)) {
  const workspacePackages = new Set([
    "infimount", "infimount_control", "infimount_core", "infimount_hosted",
    "infimount_mcp", "infimount_python",
  ]);
  const lock = fs.readFileSync(lockPath, "utf8").replace(
    /\[\[package\]\]\nname = "([^"]+)"\nversion = "[^"]+"/g,
    (block, name) => workspacePackages.has(name)
      ? block.replace(/^version = "[^"]+"/m, `version = "${version}"`)
      : block,
  );
  fs.writeFileSync(lockPath, lock, "utf8");
}

console.log(`Release version synchronized from tag ${tag} -> ${version}`);
