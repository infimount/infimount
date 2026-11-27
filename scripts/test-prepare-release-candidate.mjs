#!/usr/bin/env node
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  GitHubRequestError,
  githubRequest,
  prepareCandidate,
  validateReleaseRef,
} from "./prepare-release-candidate.mjs";

const PREFIX = "/repos/infimount/infimount";
const TAG = "v0.8.2-rc.1";
const COMMIT = "a".repeat(40);
const OTHER = "b".repeat(40);
const ANNOTATION = "c".repeat(40);
const preparationContext = () => ({
  repository: "infimount/infimount",
  serverUrl: "https://github.com",
  eventName: "workflow_dispatch",
  ref: "refs/heads/main",
  refName: "main",
  refType: "branch",
  sha: COMMIT,
  workflowRef: "infimount/infimount/.github/workflows/prepare-release-candidate.yml@refs/heads/main",
});
const releaseContext = (tag = TAG, eventName = "workflow_dispatch") => ({
  ...preparationContext(),
  eventName,
  ref: `refs/tags/${tag}`,
  refName: tag,
  refType: "tag",
  workflowRef: `infimount/infimount/.github/workflows/release.yml@refs/tags/${tag}`,
});

function fixture(settings = {}) {
  const state = {
    calls: [],
    validationCount: 0,
    mainReads: 0,
    tagReads: 0,
    releaseReads: 0,
    release: Object.hasOwn(settings, "release") ? settings.release : null,
    reference: settings.existingSha ? {
      ref: `refs/tags/${settings.tag ?? TAG}`,
      object: { type: "commit", sha: settings.existingSha },
    } : null,
    annotations: new Map(),
    dispatches: [],
  };
  const tag = settings.tag ?? TAG;
  const request = async (method, route, body) => {
    state.calls.push({ method, route, body });
    settings.beforeRequest?.({ method, route, body, state });
    if (method === "GET" && route === `${PREFIX}/git/ref/heads/main`) {
      state.mainReads += 1;
      return {
        ref: "refs/heads/main",
        object: { type: "commit", sha: state.mainReads > 1 ? settings.laterMainSha ?? COMMIT : settings.mainSha ?? COMMIT },
      };
    }
    if (method === "GET" && route === `${PREFIX}/git/ref/tags/${tag}`) {
      state.tagReads += 1;
      if (settings.tagStatus) throw new GitHubRequestError(settings.tagStatus);
      if (state.reference === null) throw new GitHubRequestError(404);
      return state.reference;
    }
    if (method === "GET" && route.startsWith(`${PREFIX}/git/tags/`)) {
      const annotated = state.annotations.get(route.slice(`${PREFIX}/git/tags/`.length));
      if (!annotated) throw new GitHubRequestError(404);
      return annotated;
    }
    if (method === "GET" && route === `${PREFIX}/releases/tags/${tag}`) {
      state.releaseReads += 1;
      if (settings.releaseStatus) throw new GitHubRequestError(settings.releaseStatus);
      if (state.release === null) throw new GitHubRequestError(404);
      return state.release;
    }
    if (method === "POST" && route === `${PREFIX}/git/tags`) {
      assert.deepEqual(body, { tag, message: `Infimount ${tag.slice(1)}`, object: COMMIT, type: "commit" });
      const annotated = { sha: ANNOTATION, tag, object: { type: "commit", sha: COMMIT } };
      state.annotations.set(ANNOTATION, annotated);
      return annotated;
    }
    if (method === "POST" && route === `${PREFIX}/git/refs`) {
      assert.deepEqual(body, { ref: `refs/tags/${tag}`, sha: ANNOTATION });
      if (settings.concurrentSha) {
        state.reference = { ref: `refs/tags/${tag}`, object: { type: "commit", sha: settings.concurrentSha } };
        throw new GitHubRequestError(422);
      }
      state.reference = { ref: body.ref, object: { type: "tag", sha: body.sha } };
      return state.reference;
    }
    if (method === "POST" && route === `${PREFIX}/actions/workflows/release.yml/dispatches`) {
      assert.deepEqual(body, { ref: tag, inputs: { expected_sha: COMMIT } });
      state.dispatches.push(body);
      return null;
    }
    throw new Error(`Unexpected fixture request: ${method} ${route}`);
  };
  const options = {
    context: preparationContext(),
    tag,
    expectedSha: COMMIT,
    localSha: COMMIT,
    version: tag.slice(1),
    request,
    validateCheckout: async () => { state.validationCount += 1; },
    publish: true,
  };
  return { state, options, request };
}

const writes = (state) => state.calls.filter(({ method }) => method !== "GET");
const verifyRelease = (item, overrides = {}) => validateReleaseRef({
  context: releaseContext(),
  localSha: COMMIT,
  expectedSha: COMMIT,
  request: item.request,
  ...overrides,
});

test("read-only preparation validates exact source and never creates a tag or dispatch", async () => {
  const item = fixture();
  const result = await prepareCandidate({ ...item.options, publish: false });
  assert.equal(result.status, "validated");
  assert.equal(result.source_commit, COMMIT);
  assert.equal(item.state.validationCount, 1);
  assert.equal(item.state.releaseReads, 1);
  assert.deepEqual(writes(item.state), []);
});

test("publication creates one annotated immutable tag and dispatches its exact approved SHA", async () => {
  const item = fixture();
  const result = await prepareCandidate(item.options);
  assert.equal(result.status, "release_dispatched");
  assert.equal(item.state.mainReads, 2);
  assert.equal(item.state.releaseReads, 2);
  assert.equal(item.state.tagReads, 2);
  assert.deepEqual(writes(item.state).map(({ route }) => route), [
    `${PREFIX}/git/tags`, `${PREFIX}/git/refs`, `${PREFIX}/actions/workflows/release.yml/dispatches`,
  ]);
  assert.deepEqual(item.state.dispatches, [{ ref: TAG, inputs: { expected_sha: COMMIT } }]);
});

test("existing identical tag and failed draft can be retried without moving the tag", async () => {
  const item = fixture({ existingSha: COMMIT, release: { tag_name: TAG, draft: true } });
  assert.equal((await prepareCandidate(item.options)).existing_tag, true);
  assert.deepEqual(writes(item.state).map(({ route }) => route), [`${PREFIX}/actions/workflows/release.yml/dispatches`]);
});

test("wrong repository, ref, event, workflow, SHA, tag or prepared version never writes", async (suite) => {
  const cases = [
    ["repository", { context: { ...preparationContext(), repository: "someone/infimount" } }],
    ["server", { context: { ...preparationContext(), serverUrl: "https://example.test" } }],
    ["topic ref", { context: { ...preparationContext(), ref: "refs/heads/topic" } }],
    ["tag ref", { context: { ...preparationContext(), ref: `refs/tags/${TAG}`, refType: "tag" } }],
    ["ref name", { context: { ...preparationContext(), refName: "topic" } }],
    ["event", { context: { ...preparationContext(), eventName: "pull_request" } }],
    ["workflow source", { context: { ...preparationContext(), workflowRef: "infimount/infimount/.github/workflows/prepare-release-candidate.yml@refs/heads/topic" } }],
    ["abbreviated SHA", { expectedSha: COMMIT.slice(0, 12) }],
    ["different expected SHA", { expectedSha: OTHER }],
    ["different event SHA", { context: { ...preparationContext(), sha: OTHER } }],
    ["different checkout", { localSha: OTHER }],
    ["wrong version", { version: "0.8.1" }],
    ["stable tag", { tag: "v0.8.2" }],
    ["beta tag", { tag: "v0.8.2-beta.1" }],
    ["zero rc", { tag: "v0.8.2-rc.0" }],
    ["leading zero", { tag: "v0.8.2-rc.01" }],
    ["build metadata", { tag: "v0.8.2-rc.1+local" }],
    ["command substitution", { tag: "v0.8.2-rc.1$(false)" }],
    ["newline", { tag: `${TAG}\n` }],
    ["MSI overflow", { tag: "v0.8.2-rc.65536", version: "0.8.2-rc.65536" }],
  ];
  for (const [name, changed] of cases) {
    await suite.test(name, async () => {
      const item = fixture();
      await assert.rejects(prepareCandidate({ ...item.options, ...changed }));
      assert.deepEqual(item.state.calls, []);
    });
  }
});

test("failed read-only checkout validation stops before any API request", async () => {
  const item = fixture();
  await assert.rejects(prepareCandidate({ ...item.options, validateCheckout: () => { throw new Error("docs mismatch"); } }), /docs mismatch/);
  assert.deepEqual(item.state.calls, []);
});

test("main drift and an existing different tag are rejected before mutations", async (suite) => {
  for (const settings of [{ mainSha: OTHER }, { laterMainSha: OTHER }, { existingSha: OTHER }, { tagStatus: 403 }]) {
    await suite.test(JSON.stringify(settings), async () => {
      const item = fixture(settings);
      await assert.rejects(prepareCandidate(item.options));
      assert.deepEqual(writes(item.state), []);
    });
  }
});

test("published and unknown release states or failed release lookups stop all writes", async (suite) => {
  for (const settings of [
    { release: { tag_name: TAG, draft: false } },
    { release: { tag_name: TAG } },
    { release: { tag_name: TAG, draft: "true" } },
    { release: { tag_name: "v0.8.2-rc.2", draft: true } },
    { releaseStatus: 401 }, { releaseStatus: 403 }, { releaseStatus: 429 }, { releaseStatus: 500 },
  ]) {
    await suite.test(JSON.stringify(settings), async () => {
      const item = fixture({ existingSha: COMMIT, ...settings });
      await assert.rejects(prepareCandidate(item.options));
      assert.deepEqual(writes(item.state), []);
      await assert.rejects(verifyRelease(item));
      await assert.rejects(verifyRelease(item, { beforePublish: true }));
      assert.deepEqual(writes(item.state), []);
    });
  }
});

test("publication racing with tag creation is detected before dispatch", async () => {
  const item = fixture({
    beforeRequest: ({ method, route, state }) => {
      if (method === "GET" && route === `${PREFIX}/releases/tags/${TAG}` && state.releaseReads === 1) {
        state.release = { tag_name: TAG, draft: false };
      }
    },
  });
  await assert.rejects(prepareCandidate(item.options), /already published/);
  assert.equal(item.state.dispatches.length, 0);
  assert.equal(writes(item.state).length, 2);
});

test("a concurrent identical tag is recoverable but a different tag is never moved", async () => {
  const same = fixture({ concurrentSha: COMMIT });
  assert.equal((await prepareCandidate(same.options)).status, "release_dispatched");
  const other = fixture({ concurrentSha: OTHER });
  await assert.rejects(prepareCandidate(other.options), /different commit/);
  assert.equal(other.state.dispatches.length, 0);
  assert(writes(other.state).every(({ method }) => method === "POST"));
});

test("canonical signing guard verifies the real tag and exact approved merged-main SHA", async () => {
  const item = fixture({ existingSha: COMMIT });
  assert.equal((await verifyRelease(item)).status, "tag_verified");
  assert.equal(item.state.mainReads, 1);
  assert.deepEqual(writes(item.state), []);
});

test("manual Release cannot switch commits or use a topic, branch, stable tag or missing tag", async (suite) => {
  const cases = [
    ["expected SHA missing", {}, { expectedSha: undefined }],
    ["expected SHA differs", {}, { expectedSha: OTHER }],
    ["main differs", { mainSha: OTHER }, {}],
    ["tag differs", { existingSha: OTHER }, {}],
    ["checkout differs", {}, { localSha: OTHER }],
    ["tag missing", { existingSha: null }, {}],
    ["wrong repository", {}, { context: { ...releaseContext(), repository: "someone/infimount" } }],
    ["branch context", {}, { context: { ...releaseContext(), ref: "refs/heads/main", refType: "branch" } }],
    ["wrong workflow", {}, { context: { ...releaseContext(), workflowRef: "infimount/infimount/.github/workflows/release.yml@refs/heads/main" } }],
    ["stable dispatch", { tag: "v0.8.2" }, { context: releaseContext("v0.8.2") }],
  ];
  for (const [name, settings, changed] of cases) {
    await suite.test(name, async () => {
      const item = fixture({ existingSha: COMMIT, ...settings });
      await assert.rejects(verifyRelease(item, changed));
      assert.deepEqual(writes(item.state), []);
    });
  }
});

test("publisher retains the approved commit even when main advances during builds", async () => {
  const item = fixture({ existingSha: COMMIT, mainSha: OTHER, release: { tag_name: TAG, draft: true } });
  assert.equal((await verifyRelease(item, { beforePublish: true })).status, "publication_verified");
  assert.equal(item.state.mainReads, 0);
  await assert.rejects(verifyRelease(item, { beforePublish: true, expectedSha: OTHER }));
  item.state.reference.object.sha = OTHER;
  await assert.rejects(verifyRelease(item, { beforePublish: true }));
  assert.deepEqual(writes(item.state), []);
});

test("existing push-tag stable releases retain the canonical path but published reruns fail", async () => {
  const tag = "v0.8.2";
  const item = fixture({ tag, existingSha: COMMIT });
  assert.equal((await verifyRelease(item, { context: releaseContext(tag, "push"), expectedSha: undefined })).status, "tag_verified");
  assert.equal(item.state.mainReads, 0);
  item.state.release = { tag_name: tag, draft: false };
  await assert.rejects(verifyRelease(item, { context: releaseContext(tag, "push"), expectedSha: undefined }), /already published/);
});

test("GitHub transport accepts documented dispatch statuses and never forwards API error bodies", async () => {
  for (const status of [200, 204]) {
    const request = githubRequest("fixture-token", async (url, options) => {
      assert.equal(url, `https://api.github.com${PREFIX}/actions/workflows/release.yml/dispatches`);
      assert.equal(options.redirect, "error");
      assert.equal(options.headers.Authorization, "Bearer fixture-token");
      assert(options.signal instanceof AbortSignal);
      return new Response(status === 200 ? JSON.stringify({ workflow_run_id: 123 }) : null, { status });
    });
    await request("POST", `${PREFIX}/actions/workflows/release.yml/dispatches`, { ref: TAG, inputs: { expected_sha: COMMIT } });
  }
  const request = githubRequest("fixture-token", async () => new Response("untrusted-response-secret", { status: 403 }));
  await assert.rejects(request("GET", `${PREFIX}/git/ref/heads/main`), (error) => {
    assert(error instanceof GitHubRequestError);
    assert.equal(error.status, 403);
    assert(!error.message.includes("untrusted-response-secret"));
    return true;
  });
});

test("GitHub transport bounds bodies and rejects unsupported destinations or mutations", async () => {
  let calls = 0;
  const request = githubRequest("fixture-token", async () => {
    calls += 1;
    return new Response(" ".repeat(64 * 1024 + 1), { status: 200 });
  });
  await assert.rejects(request("GET", `${PREFIX}/git/ref/heads/main`), /size limit/);
  for (const [method, route] of [["PATCH", `${PREFIX}/git/refs/tags/${TAG}`], ["DELETE", `${PREFIX}/git/refs/tags/${TAG}`], ["GET", "https://example.test/token"]]) {
    await assert.rejects(request(method, route), /unsupported/);
  }
  assert.equal(calls, 1);
});

test("workflow permissions, ordering and canonical release gates retain the intended boundaries", () => {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const prepare = fs.readFileSync(path.join(root, ".github/workflows/prepare-release-candidate.yml"), "utf8");
  const release = fs.readFileSync(path.join(root, ".github/workflows/release.yml"), "utf8");
  assert.match(prepare, /permissions:\n  contents: read/);
  assert.match(prepare, /create-tag-and-dispatch:[\s\S]*needs: \[validate\]/);
  assert.match(prepare, /permissions:\n      contents: write\n      actions: write/);
  assert.match(prepare, /github\.repository == 'infimount\/infimount' && github\.ref == 'refs\/heads\/main'/);
  assert.equal((prepare.match(/persist-credentials: false/g) ?? []).length, 2);
  assert.doesNotMatch(prepare, /^  (?:push|pull_request|schedule):/m);
  assert.match(release, /workflow_dispatch:\n    inputs:\n      expected_sha:/);
  const signing = release.slice(release.indexOf("  release-signing-preflight:"), release.indexOf("  release-frontend-gate:"));
  assert(signing.indexOf("check-release-ref") < signing.indexOf("Require updater signing and report platform signing status"));
  assert.match(signing, /EXPECTED_SHA: \$\{\{ inputs\.expected_sha \}\}/);
  const publication = release.slice(release.indexOf("  create-release:"));
  assert.match(publication, /needs: \[build-linux, build-macos, build-windows, release-signing-preflight, release-hosted-gate, release-python-gate\]/);
  for (const [guard, mutation] of [
    ["Require unpublished release before registry writes", "Stage the exact qualified native images and combined digest"],
    ["Require unpublished release before draft writes", "Create validated draft release"],
    ["Require the same unpublished release before final promotion", "Promote the validated hosted image to its release version"],
  ]) {
    const before = publication.indexOf(guard);
    const after = publication.indexOf(mutation);
    assert(before >= 0 && after > before);
    assert(publication.slice(before, after).includes("check-publish-ref"));
  }
});
