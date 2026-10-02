#!/usr/bin/env node

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

export const RC10_VERSION = "0.8.1-rc.10";
export const RC10_COMMIT = "86859130f93757d6893bc128e3955906a4f0130e";
export const STABLE_VERSION = "0.8.0";
export const STORAGE_ID = "11111111-1111-4111-8111-111111111111";
export const STABLE_WORKSPACE_ID = "22222222-2222-4222-8222-222222222222";
export const STORAGE_NAME = "RC10 Stable Upgrade Storage";
export const STABLE_WORKSPACE_NAME = "Stable retained workspace";
export const STABLE_WORKSPACE_ROOT = "/stable-workspace";
export const REAL_WORKSPACE_NAME = "RC10 real pilot";
export const REAL_WORKSPACE_ROOT = "/agent-workspaces/rc10-real-pilot";
export const READ_TOOLS = [
  "list_dir",
  "list_versions",
  "read_file",
  "read_file_version",
  "search_paths",
  "stat_path",
].sort();

const FIXTURE_TIME = "2026-09-05T06:37:05Z";
const BASELINE_REL = ".rc10-real-pilot/upgrade-baseline.json";

function fail(message) {
  console.error(`RC10 final pilot state check failed: ${message}`);
  process.exit(1);
}

function arg(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function requireHome() {
  const raw = arg("--home") || process.env.INFIMOUNT_RC10_PILOT_HOME;
  if (!raw) fail("--home is required");
  const home = path.resolve(raw);
  if (home === path.parse(home).root) fail("unsafe pilot home");
  return home;
}

function sha256Bytes(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function sha256File(file) {
  return sha256Bytes(fs.readFileSync(file));
}

function canonicalJson(value) {
  if (Array.isArray(value)) return value.map(canonicalJson);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value).sort().map((key) => [key, canonicalJson(value[key])]),
    );
  }
  return value;
}

function hashCanonicalJson(value) {
  return sha256Bytes(Buffer.from(JSON.stringify(canonicalJson(value))));
}

function namespaceFingerprint(root) {
  const config = { root };
  const descriptor = {
    version: 1,
    backend: "local",
    authority: "local",
    container: "",
    root,
    canonicalPublicConfigSha256: hashCanonicalJson(config),
  };
  return hashCanonicalJson(descriptor);
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
}

function configFiles(home) {
  const dir = path.join(home, ".infimount");
  return {
    dir,
    storages: path.join(dir, "storages.json"),
    workspaces: path.join(dir, "workspaces.json"),
    mcp: path.join(dir, "mcp_settings.json"),
    app: path.join(dir, "app_settings.json"),
  };
}

function confirmationRules() {
  return {
    require_for_write: true,
    require_for_overwrite: true,
    require_for_delete: true,
    require_for_version_delete: true,
    require_for_presign: true,
    require_for_cross_storage_copy: true,
  };
}

function seedUpgrade(home) {
  const files = configFiles(home);
  const baseline = path.join(home, BASELINE_REL);
  if (fs.existsSync(files.dir) || fs.existsSync(baseline)) {
    fail("pilot home already contains Infimount upgrade state");
  }

  fs.mkdirSync(home, { recursive: true, mode: 0o700 });
  const canonicalRoot = fs.realpathSync(home);
  const fingerprint = namespaceFingerprint(canonicalRoot);
  const ruleId = `workspace:${STABLE_WORKSPACE_ID}`;

  fs.mkdirSync(path.join(home, STABLE_WORKSPACE_ROOT.replace(/^\/+/, "")), {
    recursive: true,
    mode: 0o700,
  });
  fs.writeFileSync(
    path.join(home, "stable-upgrade-marker.txt"),
    "stable v0.8.0 retained marker\n",
  );

  const storage = {
    schema_version: 2,
    id: STORAGE_ID,
    name: STORAGE_NAME,
    backend: "local",
    config: { root: canonicalRoot },
    secret_fields: [],
    enabled: true,
    mcp_exposed: true,
    read_only: false,
    mcp_policy: {
      version: 2,
      default_access: "none",
      rules: [
        {
          id: ruleId,
          prefix: STABLE_WORKSPACE_ROOT,
          access: "read_only",
          source: { kind: "workspace", workspace_id: STABLE_WORKSPACE_ID },
        },
      ],
      denied_paths: [],
      confirmation_rules: confirmationRules(),
    },
    revision: 2,
    created_at: FIXTURE_TIME,
    updated_at: FIXTURE_TIME,
  };

  const workspace = {
    id: STABLE_WORKSPACE_ID,
    schemaVersion: 2,
    storageId: STORAGE_ID,
    name: STABLE_WORKSPACE_NAME,
    rootPath: STABLE_WORKSPACE_ROOT,
    templateId: "custom",
    accessProfile: "read_only",
    policyRuleId: ruleId,
    storageNamespaceFingerprint: fingerprint,
    createdAt: FIXTURE_TIME,
    updatedAt: FIXTURE_TIME,
    memoryFiles: [],
    checkpointIds: [],
  };

  writeJson(files.storages, [storage]);
  writeJson(files.workspaces, {
    schemaVersion: 1,
    revision: 1,
    workspaces: [workspace],
  });
  writeJson(files.mcp, {
    schemaVersion: 2,
    enabled: false,
    transport: "stdio",
    bindAddress: "127.0.0.1",
    port: 7331,
    enabledTools: READ_TOOLS,
    securityBaselineVersion: 2,
  });
  writeJson(files.app, {
    onboardingCompleted: true,
    onboardingSkipped: false,
    onboardingCompletedAt: FIXTURE_TIME,
    onboardingSkippedAt: null,
    wizardStep: null,
    wizardCompletedSteps: [],
    telemetryConsent: "denied",
    localEventPersistence: false,
  });

  writeJson(baseline, {
    schemaVersion: 1,
    stableVersion: STABLE_VERSION,
    candidateVersion: RC10_VERSION,
    candidateCommit: RC10_COMMIT,
    storageId: STORAGE_ID,
    workspaceId: STABLE_WORKSPACE_ID,
    namespaceFingerprint: fingerprint,
    hashes: {
      storages: sha256File(files.storages),
      workspaces: sha256File(files.workspaces),
      mcp: sha256File(files.mcp),
      app: sha256File(files.app),
    },
  });

  console.log("upgrade_fixture_seeded=yes");
  console.log("stable_storage_count=1");
  console.log("stable_workspace_count=1");
  console.log(`stable_namespace_fingerprint=${fingerprint}`);
}

function assertUpgrade(home) {
  const files = configFiles(home);
  const baselineFile = path.join(home, BASELINE_REL);
  if (!fs.existsSync(baselineFile)) fail("upgrade baseline is missing");
  const baseline = JSON.parse(fs.readFileSync(baselineFile, "utf8"));

  const current = {
    storages: sha256File(files.storages),
    workspaces: sha256File(files.workspaces),
    mcp: sha256File(files.mcp),
    app: sha256File(files.app),
  };
  for (const key of Object.keys(current)) {
    if (current[key] !== baseline.hashes[key]) {
      fail(`${key} changed across package installer-over-install`);
    }
  }

  const storages = JSON.parse(fs.readFileSync(files.storages, "utf8"));
  const workspaceDoc = JSON.parse(fs.readFileSync(files.workspaces, "utf8"));
  if (!Array.isArray(storages) || storages.length !== 1 || storages[0].id !== STORAGE_ID) {
    fail("stable storage registry was not retained");
  }
  const workspaces = workspaceDoc.workspaces || [];
  if (workspaces.length !== 1 || workspaces[0].id !== STABLE_WORKSPACE_ID) {
    fail("stable workspace registry was not retained");
  }
  if (workspaces[0].storageNamespaceFingerprint !== baseline.namespaceFingerprint) {
    fail("stable workspace namespace binding changed");
  }
  if (
    fs.readFileSync(path.join(home, "stable-upgrade-marker.txt"), "utf8")
    !== "stable v0.8.0 retained marker\n"
  ) {
    fail("stable storage bytes changed");
  }

  console.log("upgrade_configuration_retained=yes");
  console.log("upgrade_storage_registry_retained=yes");
  console.log("upgrade_workspace_registry_retained=yes");
  console.log("upgrade_package_state_exact=yes");
}

function summary(home) {
  const files = configFiles(home);
  const storages = JSON.parse(fs.readFileSync(files.storages, "utf8"));
  const workspaceDoc = JSON.parse(fs.readFileSync(files.workspaces, "utf8"));
  console.log(JSON.stringify({
    storageCount: storages.length,
    workspaceCount: (workspaceDoc.workspaces || []).length,
    candidate: RC10_VERSION,
    candidateCommit: RC10_COMMIT,
  }, null, 2));
}

const command = process.argv[2];
const home = requireHome();
if (command === "seed-upgrade") {
  seedUpgrade(home);
} else if (command === "assert-upgrade") {
  assertUpgrade(home);
} else if (command === "summary") {
  summary(home);
} else {
  fail("usage: rc10-final-pilot-state.mjs <seed-upgrade|assert-upgrade|summary> --home <path>");
}
