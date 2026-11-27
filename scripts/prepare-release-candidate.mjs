#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";
import { canonicalVersionFromTag, windowsMsiVersion } from "./release-version-utils.mjs";

const REPOSITORY = "infimount/infimount";
const REPOSITORY_PATH = `/repos/${REPOSITORY}`;
const PREPARATION_WORKFLOW = "prepare-release-candidate.yml";
const SHA = /^[0-9a-f]{40}$/;
const CANDIDATE = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)-rc\.([1-9]\d*)$/;

const requireCondition = (condition, message) => {
  if (!condition) throw new Error(message);
};

export class GitHubRequestError extends Error {
  constructor(status) {
    super(`GitHub request failed with HTTP ${status}`);
    this.status = status;
  }
}

export function validatePreparationContext({ context, tag, expectedSha, localSha, version }) {
  requireCondition(context.repository === REPOSITORY, "candidate preparation requires the canonical repository");
  requireCondition(context.serverUrl === "https://github.com", "candidate preparation requires github.com");
  requireCondition(context.eventName === "workflow_dispatch", "candidate preparation must be manually dispatched");
  requireCondition(context.ref === "refs/heads/main" && context.refName === "main" && context.refType === "branch", "candidate preparation must run from main");
  requireCondition(
    context.workflowRef === `${REPOSITORY}/.github/workflows/${PREPARATION_WORKFLOW}@refs/heads/main`,
    "candidate preparation must use the canonical workflow on main",
  );
  requireCondition(typeof tag === "string" && tag.length <= 100 && CANDIDATE.test(tag), "candidate tag must be vX.Y.Z-rc.N with positive N");
  windowsMsiVersion(tag);
  requireCondition(typeof expectedSha === "string" && SHA.test(expectedSha), "expected SHA must be a full lowercase commit SHA");
  requireCondition(context.sha === expectedSha && localSha === expectedSha, "expected SHA must match the workflow and checked-out commit");
  requireCondition(version === tag.slice(1), "the checked-out version must already match the candidate tag");
  return { tag, sha: expectedSha };
}

async function requireCurrentMain(request, expectedSha) {
  const reference = await request("GET", `${REPOSITORY_PATH}/git/ref/heads/main`);
  requireCondition(
    reference?.ref === "refs/heads/main" && reference.object?.type === "commit" && reference.object.sha === expectedSha,
    "expected SHA is no longer the exact main head",
  );
}

async function requireUnpublishedRelease(request, tag) {
  let release;
  try {
    release = await request("GET", `${REPOSITORY_PATH}/releases/tags/${tag}`);
  } catch (error) {
    if (error instanceof GitHubRequestError && error.status === 404) return;
    throw error;
  }
  requireCondition(
    release?.tag_name === tag && release.draft === true,
    "release is already published or its draft state cannot be verified",
  );
}

export async function resolveReleaseTag(request, tag, allowMissing = false) {
  let reference;
  try {
    reference = await request("GET", `${REPOSITORY_PATH}/git/ref/tags/${tag}`);
  } catch (error) {
    if (allowMissing && error instanceof GitHubRequestError && error.status === 404) return null;
    throw error;
  }
  requireCondition(reference?.ref === `refs/tags/${tag}`, "GitHub returned a different tag reference");
  requireCondition(SHA.test(reference.object?.sha ?? ""), "tag object has an invalid SHA");
  if (reference.object.type === "commit") return reference.object.sha;
  requireCondition(reference.object.type === "tag", "release tag must point to a commit or annotated tag");
  const annotated = await request("GET", `${REPOSITORY_PATH}/git/tags/${reference.object.sha}`);
  requireCondition(
    annotated?.sha === reference.object.sha && annotated.tag === tag
      && annotated.object?.type === "commit" && SHA.test(annotated.object.sha ?? ""),
    "annotated release tag must directly identify the matching commit",
  );
  return annotated.object.sha;
}

export async function prepareCandidate(options) {
  const { request, validateCheckout, publish = false } = options;
  const { tag, sha } = validatePreparationContext(options);
  requireCondition(typeof validateCheckout === "function", "read-only checkout validation is required");
  await validateCheckout(tag);
  await requireCurrentMain(request, sha);
  const existing = await resolveReleaseTag(request, tag, true);
  requireCondition(existing === null || existing === sha, "candidate tag already identifies a different commit");
  await requireUnpublishedRelease(request, tag);
  if (!publish) return { tag, source_commit: sha, status: "validated", existing_tag: existing !== null };

  // The write-capable job repeats all read-only checks. Check main again just
  // before creating anything, and never update or delete an existing reference.
  await requireCurrentMain(request, sha);
  if (existing === null) {
    const annotated = await request("POST", `${REPOSITORY_PATH}/git/tags`, {
      tag,
      message: `Infimount ${tag.slice(1)}`,
      object: sha,
      type: "commit",
    });
    requireCondition(
      SHA.test(annotated?.sha ?? "") && annotated.tag === tag
        && annotated.object?.type === "commit" && annotated.object.sha === sha,
      "created tag object does not match the approved candidate",
    );
    try {
      await request("POST", `${REPOSITORY_PATH}/git/refs`, { ref: `refs/tags/${tag}`, sha: annotated.sha });
    } catch (error) {
      // A concurrent create can only be accepted if the exact commit is now
      // present. Other failures stay fatal and an existing tag is never moved.
      if (!(error instanceof GitHubRequestError) || error.status !== 422) throw error;
      requireCondition(await resolveReleaseTag(request, tag) === sha, "concurrent tag creation identifies a different commit");
    }
  }
  requireCondition(await resolveReleaseTag(request, tag) === sha, "candidate tag changed before release dispatch");
  await requireUnpublishedRelease(request, tag);
  await request("POST", `${REPOSITORY_PATH}/actions/workflows/release.yml/dispatches`, {
    ref: tag,
    inputs: { expected_sha: sha },
  });
  return { tag, source_commit: sha, status: "release_dispatched", existing_tag: existing !== null };
}

export async function validateReleaseRef({ context, localSha, expectedSha, request, beforePublish = false }) {
  requireCondition(context.repository === REPOSITORY && context.serverUrl === "https://github.com", "release requires the canonical GitHub repository");
  requireCondition(["push", "workflow_dispatch"].includes(context.eventName), "release requires a tag push or explicit candidate dispatch");
  const tag = context.refName;
  requireCondition(typeof tag === "string" && tag.startsWith("v") && tag.length <= 100, "release requires an exact version tag");
  canonicalVersionFromTag(tag);
  requireCondition(context.refType === "tag" && context.ref === `refs/tags/${tag}`, "release requires an actual matching tag ref");
  requireCondition(
    context.workflowRef === `${REPOSITORY}/.github/workflows/release.yml@refs/tags/${tag}`,
    "release must use the canonical workflow at the selected tag",
  );
  if (context.eventName === "workflow_dispatch") {
    requireCondition(CANDIDATE.test(tag), "manual release dispatch supports rc tags only");
    requireCondition(SHA.test(expectedSha ?? "") && expectedSha === context.sha, "manual release must retain the explicitly approved commit SHA");
    // Main may advance while the qualified packages build. The signing gate
    // proves main membership before builds, then the publisher retains that
    // exact approved SHA and tag instead of switching to a newer main commit.
    if (!beforePublish) await requireCurrentMain(request, expectedSha);
  }
  requireCondition(SHA.test(context.sha ?? "") && context.sha === localSha, "release checkout must match the event commit");
  requireCondition(await resolveReleaseTag(request, tag) === localSha, "release tag must resolve to the exact checked-out commit");
  await requireUnpublishedRelease(request, tag);
  return { tag, source_commit: localSha, status: beforePublish ? "publication_verified" : "tag_verified" };
}

export function githubRequest(token, fetchImplementation = fetch) {
  requireCondition(typeof token === "string" && token.length > 0 && token.length <= 8192 && !/\s/.test(token), "GITHUB_TOKEN is required");
  return async (method, route, body) => {
    requireCondition(["GET", "POST"].includes(method) && route.startsWith(`${REPOSITORY_PATH}/`) && !/[\s?#]/.test(route), "unsupported GitHub request");
    const response = await fetchImplementation(`https://api.github.com${route}`, {
      method,
      headers: {
        Accept: "application/vnd.github+json",
        Authorization: `Bearer ${token}`,
        "Content-Type": "application/json",
        "X-GitHub-Api-Version": "2022-11-28",
      },
      body: body === undefined ? undefined : JSON.stringify(body),
      redirect: "error",
      signal: AbortSignal.timeout(15_000),
    });
    const expectedStatuses = method === "GET" ? [200] : route.endsWith("/dispatches") ? [200, 204] : [201];
    if (!expectedStatuses.includes(response.status)) {
      await response.body?.cancel();
      throw new GitHubRequestError(response.status);
    }
    if (response.status === 204) return null;
    const chunks = [];
    let length = 0;
    requireCondition(response.body !== null, "GitHub response has no body");
    for await (const chunk of response.body) {
      length += chunk.byteLength;
      requireCondition(length <= 64 * 1024, "GitHub response exceeds the size limit");
      chunks.push(chunk);
    }
    return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(Buffer.concat(chunks)));
  };
}

function workflowContext() {
  return {
    repository: process.env.GITHUB_REPOSITORY,
    serverUrl: process.env.GITHUB_SERVER_URL,
    eventName: process.env.GITHUB_EVENT_NAME,
    ref: process.env.GITHUB_REF,
    refName: process.env.GITHUB_REF_NAME,
    refType: process.env.GITHUB_REF_TYPE,
    sha: process.env.GITHUB_SHA,
    workflowRef: process.env.GITHUB_WORKFLOW_REF,
  };
}

function validatePreparedCheckout(repository, tag) {
  execFileSync("git", ["diff", "--quiet"], { cwd: repository, stdio: "inherit" });
  execFileSync("git", ["diff", "--cached", "--quiet"], { cwd: repository, stdio: "inherit" });
  for (const [name, args] of [
    ["check-release-consistency.mjs", [tag]],
    ["check-feature-docs.mjs", []],
  ]) {
    execFileSync(process.execPath, [path.join(repository, "scripts", name), ...args], {
      cwd: repository,
      env: { ...process.env, INFIMOUNT_REQUIRE_PUBLISHED_RELEASE: "1" },
      stdio: "inherit",
    });
  }
}

async function main() {
  requireCondition(process.argv.length === 3, "pass validate, publish, check-release-ref, or check-publish-ref");
  const mode = process.argv[2];
  requireCondition(["validate", "publish", "check-release-ref", "check-publish-ref"].includes(mode), "unknown release preparation mode");
  const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const localSha = execFileSync("git", ["rev-parse", "HEAD"], { cwd: repository, encoding: "utf8" }).trim();
  const context = workflowContext();
  const request = githubRequest(process.env.GITHUB_TOKEN);
  const result = mode === "check-release-ref" || mode === "check-publish-ref"
    ? await validateReleaseRef({
      context,
      localSha,
      expectedSha: process.env.EXPECTED_SHA,
      request,
      beforePublish: mode === "check-publish-ref",
    })
    : await prepareCandidate({
      context,
      tag: process.env.CANDIDATE_TAG,
      expectedSha: process.env.EXPECTED_SHA,
      localSha,
      version: JSON.parse(fs.readFileSync(path.join(repository, "package.json"), "utf8")).version,
      request,
      validateCheckout: (tag) => validatePreparedCheckout(repository, tag),
      publish: mode === "publish",
    });
  console.log(JSON.stringify(result, null, 2));
  if (process.env.GITHUB_STEP_SUMMARY) {
    fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### Candidate release automation\n\nTag: \`${result.tag}\`\n\nSource: \`${result.source_commit}\`\n\nResult: \`${result.status}\`\n`);
  }
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  main().catch((error) => {
    console.error(`Release automation failed: ${error.message}`);
    process.exitCode = 1;
  });
}
