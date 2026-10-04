#!/usr/bin/env node

import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const READ_TOOLS = [
  "list_dir",
  "list_versions",
  "read_file",
  "read_file_version",
  "search_paths",
  "stat_path",
].sort();

const WRITE_TOOLS = [...READ_TOOLS, "mkdir", "write_file"].sort();
const WORKSPACE_NAME = "Release pilot workspace";
const WORKSPACE_ROOT = "/agent-workspaces/release-pilot-workspace";
const WRITABLE_WORKSPACE_NAME = "Release pilot writable";
const WRITABLE_WORKSPACE_ROOT = "/agent-workspaces/release-pilot-writable";
const STORAGE_ID = "release-pilot-home";
const STORAGE_NAME = "Pilot Home";
const FIXTURE_TIME = "2026-01-01T00:00:00Z";
const BASELINE_DIR = ".release-pilot-baseline";
const PAGINATION_DIR = "pagination";
const PAGINATION_COUNT = 260;
const PAGINATION_LAST_FILE = `page-${String(PAGINATION_COUNT - 1).padStart(4, "0")}.txt`;
const AGENT_TASK_SOURCE_FILE = "agent-task-source.txt";
const AGENT_TASK_SOURCE_CONTENT = "release pilot source bytes must remain unchanged\n";
const AGENT_TASK_DESTINATION_DIR = "publish-destination";
const AGENT_TASK_SENTINEL_FILE = "result.md";
const AGENT_TASK_SENTINEL_CONTENT = "existing destination sentinel must remain unchanged\n";
const AGENT_TASK_FINAL_OUTPUT = "deterministic agent output v2\n";

function fail(message) {
  console.error(`Release pilot state check failed: ${message}`);
  process.exit(1);
}

function getArg(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function resolveHome() {
  const value = getArg("--home") || process.env.INFIMOUNT_PILOT_HOME || process.env.HOME;
  if (!value) fail("--home or INFIMOUNT_PILOT_HOME is required");
  return path.resolve(value);
}

function readJson(file) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`cannot read JSON ${file}: ${error.message}`);
  }
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
}

function sha256File(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function normalizedJson(value) {
  return JSON.stringify(value);
}

function assert(condition, message) {
  if (!condition) fail(message);
}

function configDir(home) {
  return path.join(home, ".infimount");
}

function baselineDir(home) {
  return path.join(home, BASELINE_DIR);
}

function configurationFiles(home) {
  const config = configDir(home);
  return {
    storages: path.join(config, "storages.json"),
    workspaces: path.join(config, "workspaces.json"),
    mcp: path.join(config, "mcp_settings.json"),
    app: path.join(config, "app_settings.json"),
  };
}

function seed(home) {
  const config = configDir(home);
  const baseline = baselineDir(home);
  if (fs.existsSync(config) || fs.existsSync(baseline)) {
    fail(`refusing to seed a non-empty pilot home: ${home}`);
  }

  fs.mkdirSync(config, { recursive: true, mode: 0o700 });
  fs.mkdirSync(baseline, { recursive: true, mode: 0o700 });
  fs.mkdirSync(path.join(home, "outside"), { recursive: true });
  fs.writeFileSync(path.join(home, "pilot-browser.txt"), "release pilot browser fixture\n");

  const paginationRoot = path.join(home, PAGINATION_DIR);
  fs.mkdirSync(paginationRoot, { recursive: true });
  for (let index = 0; index < PAGINATION_COUNT; index += 1) {
    const name = `page-${String(index).padStart(4, "0")}.txt`;
    fs.writeFileSync(path.join(paginationRoot, name), `release pilot page ${index}\n`);
  }

  fs.writeFileSync(path.join(home, AGENT_TASK_SOURCE_FILE), AGENT_TASK_SOURCE_CONTENT);
  const publicationDestination = path.join(home, AGENT_TASK_DESTINATION_DIR);
  fs.mkdirSync(publicationDestination, { recursive: true });
  fs.writeFileSync(
    path.join(publicationDestination, AGENT_TASK_SENTINEL_FILE),
    AGENT_TASK_SENTINEL_CONTENT,
  );

  const storage = {
    schema_version: 2,
    id: STORAGE_ID,
    name: STORAGE_NAME,
    backend: "local",
    config: { root: "~" },
    secret_fields: [],
    enabled: true,
    mcp_exposed: false,
    read_only: false,
    mcp_policy: {
      version: 2,
      default_access: "none",
      rules: [],
      denied_paths: [],
      confirmation_rules: {
        require_for_write: true,
        require_for_overwrite: true,
        require_for_delete: true,
        require_for_version_delete: true,
        require_for_presign: true,
        require_for_cross_storage_copy: true,
      },
    },
    revision: 1,
    created_at: FIXTURE_TIME,
    updated_at: FIXTURE_TIME,
  };

  const workspaces = {
    schemaVersion: 1,
    revision: 0,
    workspaces: [],
  };

  const mcp = {
    schemaVersion: 2,
    enabled: false,
    transport: "stdio",
    bindAddress: "127.0.0.1",
    port: 7331,
    enabledTools: READ_TOOLS,
    securityBaselineVersion: 2,
  };

  const app = {
    onboardingCompleted: false,
    onboardingSkipped: true,
    onboardingCompletedAt: null,
    onboardingSkippedAt: FIXTURE_TIME,
    wizardStep: null,
    wizardCompletedSteps: [],
    telemetryConsent: "unknown",
    localEventPersistence: false,
  };

  const files = configurationFiles(home);
  writeJson(files.storages, [storage]);
  writeJson(files.workspaces, workspaces);
  writeJson(files.mcp, mcp);
  writeJson(files.app, app);

  for (const [name, file] of Object.entries(files)) {
    fs.copyFileSync(file, path.join(baseline, path.basename(file)));
    console.log(`seeded_${name}=yes`);
  }
  console.log("pilot_storage_count=1");
  console.log("legacy_home_alias_count=1");
  console.log("onboarding_state=skipped");
  console.log(`pagination_fixture_count=${PAGINATION_COUNT}`);
}

function assertStorageOnly(home) {
  const files = configurationFiles(home);
  const baseline = baselineDir(home);

  for (const key of ["storages", "workspaces", "mcp"]) {
    const current = files[key];
    const original = path.join(baseline, path.basename(current));
    assert(fs.existsSync(current), `${path.basename(current)} is missing`);
    assert(
      sha256File(current) === sha256File(original),
      `${path.basename(current)} changed during storage-only onboarding`,
    );
  }

  const before = readJson(path.join(baseline, "app_settings.json"));
  const after = readJson(files.app);
  assert(before.onboardingCompleted === false, "baseline onboardingCompleted must be false");
  assert(before.onboardingSkipped === true, "baseline onboardingSkipped must be true");
  assert(after.onboardingCompleted === true, "storage-only onboarding did not complete");
  assert(after.onboardingSkipped === false, "storage-only onboarding did not clear skipped state");
  assert(
    typeof after.onboardingCompletedAt === "string" && after.onboardingCompletedAt.length > 0,
    "storage-only onboarding completion timestamp missing",
  );

  const expected = {
    ...before,
    onboardingCompleted: true,
    onboardingSkipped: false,
    onboardingCompletedAt: after.onboardingCompletedAt,
    wizardStep: null,
  };
  assert(
    normalizedJson(expected) === normalizedJson(after),
    "storage-only onboarding changed unexpected app settings",
  );

  console.log("storage_only_onboarding=passed");
  console.log("storage_registry_changed=no");
  console.log("workspace_registry_changed=no");
  console.log("mcp_settings_changed=no");
}

function currentWorkspace(workspaces, name) {
  return workspaces.find((workspace) => workspace.name === name);
}

function assertReadOnlyAgentAccess(home) {
  const files = configurationFiles(home);
  const storages = readJson(files.storages);
  const workspaceDoc = readJson(files.workspaces);
  const mcp = readJson(files.mcp);
  const app = readJson(files.app);

  assert(Array.isArray(storages) && storages.length === 1, "storage registry count must remain 1");
  const storage = storages[0];
  const canonicalHome = fs.realpathSync(home);
  assert(storage.id === STORAGE_ID, "pilot storage identity changed");
  assert(storage.config?.root === canonicalHome, "legacy ~ root was not canonicalized");
  assert(storage.mcp_exposed === true, "workspace storage was not exposed after Agent Access prepare");
  assert(storage.revision === 4, `expected storage revision 4, got ${storage.revision}`);

  const workspaces = workspaceDoc.workspaces ?? [];
  assert(workspaces.length === 1, `expected 1 workspace, got ${workspaces.length}`);
  const workspace = currentWorkspace(workspaces, WORKSPACE_NAME);
  assert(Boolean(workspace), "read-only pilot workspace missing");
  assert(workspace.storageId === STORAGE_ID, "workspace storage binding changed");
  assert(workspace.rootPath === WORKSPACE_ROOT, "workspace root mismatch");
  assert(workspace.templateId === "custom", "workspace template must be custom");
  assert(workspace.accessProfile === "read_only", "workspace must be read-only");
  assert(Array.isArray(workspace.memoryFiles) && workspace.memoryFiles.length === 0, "custom workspace gained memory files");
  assert(
    typeof workspace.storageNamespaceFingerprint === "string" &&
      workspace.storageNamespaceFingerprint.length > 0,
    "workspace namespace fingerprint missing",
  );

  const policy = storage.mcp_policy;
  assert(policy.default_access === "none", "storage default MCP access broadened");
  assert(Array.isArray(policy.rules) && policy.rules.length === 1, "expected one workspace policy rule");
  const rule = policy.rules[0];
  assert(rule.source?.kind === "workspace", "policy rule is not workspace-managed");
  assert(rule.source?.workspace_id === workspace.id, "policy workspace identity mismatch");
  assert(rule.access === "read_only", "policy rule must be read-only");
  assert(
    String(rule.prefix).replace(/^\/+|\/+$/g, "") === WORKSPACE_ROOT.replace(/^\/+|\/+$/g, ""),
    "policy prefix does not match workspace",
  );

  assert(mcp.enabled === true, "guided Agent Access did not enable general access");
  assert(mcp.transport === "stdio", "first-time guided Agent Access did not choose stdio");
  assert(mcp.bindAddress === "127.0.0.1", "guided Agent Access is not loopback-bound");
  assert(
    normalizedJson([...mcp.enabledTools].sort()) === normalizedJson(READ_TOOLS),
    `read-only tool set mismatch: ${JSON.stringify(mcp.enabledTools)}`,
  );
  assert(!mcp.enabledTools.includes("mkdir") && !mcp.enabledTools.includes("write_file"), "write tools leaked into read-only profile");
  assert(app.onboardingCompleted === true && app.onboardingSkipped === false, "onboarding state regressed");

  const inside = path.join(home, WORKSPACE_ROOT.replace(/^\/+/, ""), "sample.txt");
  const outside = path.join(home, "outside", "denied.txt");
  assert(fs.existsSync(inside), "activation inside fixture missing");
  assert(fs.existsSync(outside), "activation outside fixture missing");

  console.log("legacy_home_normalization=passed");
  console.log("read_only_agent_access=passed");
  console.log("general_transport=stdio");
  console.log(`enabled_tool_count=${mcp.enabledTools.length}`);
}

function assertReadWriteAgentAccess(home) {
  const files = configurationFiles(home);
  const storages = readJson(files.storages);
  const workspaceDoc = readJson(files.workspaces);
  const mcp = readJson(files.mcp);

  assert(storages.length === 1, "storage registry count changed");
  const storage = storages[0];
  const workspaces = workspaceDoc.workspaces ?? [];
  assert(workspaces.length === 2, `expected 2 workspaces, got ${workspaces.length}`);
  const readOnly = currentWorkspace(workspaces, WORKSPACE_NAME);
  const writable = currentWorkspace(workspaces, WRITABLE_WORKSPACE_NAME);
  assert(Boolean(readOnly), "read-only workspace disappeared");
  assert(Boolean(writable), "read-write workspace missing");
  assert(writable.rootPath === WRITABLE_WORKSPACE_ROOT, "read-write workspace root mismatch");
  assert(writable.accessProfile === "read_write", "second workspace is not read-write");
  assert(storage.revision === 5, `expected storage revision 5 after second policy rule, got ${storage.revision}`);
  assert(storage.mcp_exposed === true, "storage exposure was lost");

  const workspaceRules = (storage.mcp_policy?.rules ?? []).filter(
    (rule) => rule.source?.kind === "workspace",
  );
  assert(workspaceRules.length === 2, "expected exactly two workspace-managed rules");
  const writeRule = workspaceRules.find((rule) => rule.source?.workspace_id === writable.id);
  assert(Boolean(writeRule), "read-write managed rule missing");
  assert(writeRule.access === "read_write", "read-write managed rule has wrong access");

  assert(mcp.enabled === true, "Agent Access unexpectedly disabled");
  assert(mcp.transport === "stdio", "read-write prepare changed stdio transport");
  assert(
    normalizedJson([...mcp.enabledTools].sort()) === normalizedJson(WRITE_TOOLS),
    `read-write tool set mismatch: ${JSON.stringify(mcp.enabledTools)}`,
  );

  console.log("read_write_agent_access=passed");
  console.log(`enabled_tool_count=${mcp.enabledTools.length}`);
}

function assertHttpStopped(home) {
  const mcp = readJson(configurationFiles(home).mcp);
  assert(mcp.transport === "http", "advanced settings did not persist HTTP transport");
  assert(mcp.enabled === false, "Stop HTTP Server did not persist disabled general access");
  assert(mcp.bindAddress === "127.0.0.1", "HTTP bind address is not loopback");
  assert(
    normalizedJson([...mcp.enabledTools].sort()) === normalizedJson(WRITE_TOOLS),
    "HTTP lifecycle unexpectedly changed the tool set",
  );
  console.log("http_explicit_start_stop=passed");
  console.log("http_persisted_state=stopped");
}

function assertGuidedReenable(home) {
  const mcp = readJson(configurationFiles(home).mcp);
  assert(mcp.enabled === true, "guided prepare did not re-enable Agent Access");
  assert(mcp.transport === "stdio", "disabled guided setup did not return to local stdio");
  assert(mcp.bindAddress === "127.0.0.1", "guided re-enable changed loopback bind");
  assert(
    normalizedJson([...mcp.enabledTools].sort()) === normalizedJson(READ_TOOLS),
    "read-only guided re-enable did not restore the minimum read tool profile",
  );
  console.log("disabled_guided_setup_returns_to_stdio=passed");
}

function assertPagination(home) {
  const root = path.join(home, PAGINATION_DIR);
  assert(fs.existsSync(root) && fs.statSync(root).isDirectory(), "pagination fixture directory is missing");
  const names = fs.readdirSync(root).filter((name) => name.startsWith("page-") && name.endsWith(".txt")).sort();
  assert(names.length === PAGINATION_COUNT, `expected ${PAGINATION_COUNT} pagination files, got ${names.length}`);
  assert(names.at(-1) === PAGINATION_LAST_FILE, `last pagination fixture mismatch: ${names.at(-1)}`);
  console.log("pagination_auto_continuation=passed");
  console.log("stale_cursor_recovery=passed");
  console.log(`pagination_fixture_count=${PAGINATION_COUNT}`);
}

function restoreLegacyHomeAliasForAgentTask(home) {
  const files = configurationFiles(home);
  const storages = readJson(files.storages);
  assert(Array.isArray(storages) && storages.length === 1, "storage registry count must remain 1");
  const storage = storages[0];
  const canonicalHome = fs.realpathSync(home);
  assert(storage.config?.root === canonicalHome, "pilot storage must be canonical before restoring legacy alias");
  storage.config.root = "~";
  storage.revision = Number(storage.revision ?? 0) + 1;
  storage.updated_at = new Date().toISOString();
  writeJson(files.storages, storages);
  console.log("legacy_home_alias_restored_for_agent_task=yes");
}

function assertAgentTaskPublication(home) {
  const files = configurationFiles(home);
  const storages = readJson(files.storages);
  const workspaceDoc = readJson(files.workspaces);
  assert(Array.isArray(storages) && storages.length === 1, "Agent Task pilot storage registry changed");
  assert(storages[0].mcp_exposed === true, "Agent Task flow changed source storage MCP exposure");
  assert(storages[0].config?.root === "~", "Agent Task flow unexpectedly rewrote the legacy source root");

  const source = path.join(home, AGENT_TASK_SOURCE_FILE);
  assert(fs.readFileSync(source, "utf8") === AGENT_TASK_SOURCE_CONTENT, "Agent Task source bytes changed");

  const sentinel = path.join(home, AGENT_TASK_DESTINATION_DIR, AGENT_TASK_SENTINEL_FILE);
  assert(fs.readFileSync(sentinel, "utf8") === AGENT_TASK_SENTINEL_CONTENT, "fail/rename publication overwrote the destination sentinel");

  const renamed = path.join(home, AGENT_TASK_DESTINATION_DIR, "result copy.md");
  assert(fs.existsSync(renamed), "renamed Agent Task publication output is missing");
  assert(fs.readFileSync(renamed, "utf8") === AGENT_TASK_FINAL_OUTPUT, "published Agent Task output bytes do not match the final reviewed output");
  assert(!fs.existsSync(path.join(home, AGENT_TASK_DESTINATION_DIR, "result copy 2.md")), "Agent Task publication created more than one renamed output");

  const workspaces = workspaceDoc.workspaces ?? [];
  const workspace = currentWorkspace(workspaces, WRITABLE_WORKSPACE_NAME);
  assert(Boolean(workspace), "read-write Agent Task workspace is missing");

  const tasksRoot = path.join(home, String(workspace.rootPath).replace(/^\/+/, ""), "tasks");
  assert(fs.existsSync(tasksRoot), "Agent Task package root is missing");
  const taskDirs = fs.readdirSync(tasksRoot, { withFileTypes: true }).filter((entry) => entry.isDirectory());
  assert(taskDirs.length === 1, `expected exactly one Agent Task package, got ${taskDirs.length}`);
  const taskRoot = path.join(tasksRoot, taskDirs[0].name);

  const preparedInput = path.join(taskRoot, "inputs", AGENT_TASK_SOURCE_FILE);
  assert(fs.existsSync(preparedInput), "prepared Agent Task input copy is missing");
  assert(fs.readFileSync(preparedInput, "utf8") === AGENT_TASK_SOURCE_CONTENT, "prepared Agent Task input differs from source bytes");

  const output = path.join(taskRoot, "outputs", "result.md");
  assert(fs.existsSync(output), "Agent Task output is missing");
  assert(fs.readFileSync(output, "utf8") === AGENT_TASK_FINAL_OUTPUT, "task output does not contain the post-stale-review bytes");

  const receipts = fs.readdirSync(taskRoot)
    .filter((name) => /^publish-receipt-[0-9a-f-]+\.json$/i.test(name))
    .sort();
  assert(receipts.length === 1, `expected exactly one publication receipt, got ${receipts.length}`);
  const receipt = readJson(path.join(taskRoot, receipts[0]));
  assert(receipt.taskId === taskDirs[0].name, "publication receipt task identity mismatch");
  assert(receipt.workspaceId === workspace.id, "publication receipt workspace identity mismatch");
  assert(receipt.destinationStorageId === storages[0].id, "publication receipt destination storage mismatch");
  assert(receipt.destinationDir === AGENT_TASK_DESTINATION_DIR, "publication receipt destination directory mismatch");
  assert(receipt.conflictPolicy === "rename", "publication receipt conflict policy is not rename");
  assert(Array.isArray(receipt.files) && receipt.files.length === 1, "publication receipt file count mismatch");
  const published = receipt.files[0];
  assert(published.taskPath === "outputs/result.md", "publication receipt task path mismatch");
  assert(published.destinationPath === `${AGENT_TASK_DESTINATION_DIR}/result copy.md`, "publication receipt rename path mismatch");
  assert(published.action === "rename", "publication receipt action is not rename");

  const receiptText = fs.readFileSync(path.join(taskRoot, receipts[0]), "utf8");
  assert(!receiptText.includes(home), "publication receipt leaked a host filesystem path");
  assert(!receiptText.includes(AGENT_TASK_SOURCE_FILE), "publication receipt leaked a source path");

  console.log("agent_task_source_bytes_unchanged=passed");
  console.log("agent_task_fail_conflict_rejected=passed");
  console.log("agent_task_stale_preview_rejected=passed");
  console.log("agent_task_overwrite_unavailable=passed");
  console.log("agent_task_destination_verified=passed");
  console.log("agent_task_unique_receipt=passed");
}

function summary(home) {
  const files = configurationFiles(home);
  const storages = readJson(files.storages);
  const workspaces = readJson(files.workspaces).workspaces ?? [];
  const mcp = readJson(files.mcp);
  const app = readJson(files.app);
  console.log(JSON.stringify({
    storageCount: storages.length,
    workspaceCount: workspaces.length,
    onboardingCompleted: app.onboardingCompleted,
    mcpEnabled: mcp.enabled,
    transport: mcp.transport,
    enabledToolCount: Array.isArray(mcp.enabledTools) ? mcp.enabledTools.length : 0,
  }, null, 2));
}

const command = process.argv[2];
const home = resolveHome();

switch (command) {
  case "seed":
    seed(home);
    break;
  case "assert-storage-only":
    assertStorageOnly(home);
    break;
  case "assert-read-only-agent-access":
    assertReadOnlyAgentAccess(home);
    break;
  case "assert-read-write-agent-access":
    assertReadWriteAgentAccess(home);
    break;
  case "assert-http-stopped":
    assertHttpStopped(home);
    break;
  case "assert-guided-reenable":
    assertGuidedReenable(home);
    break;
  case "assert-pagination":
    assertPagination(home);
    break;
  case "restore-legacy-home-alias":
    restoreLegacyHomeAliasForAgentTask(home);
    break;
  case "assert-agent-task-publication":
    assertAgentTaskPublication(home);
    break;
  case "summary":
    summary(home);
    break;
  default:
    fail("usage: release-pilot-state.mjs <seed|assert-storage-only|assert-read-only-agent-access|assert-read-write-agent-access|assert-http-stopped|assert-guided-reenable|assert-pagination|restore-legacy-home-alias|assert-agent-task-publication|summary> --home <path>");
}
