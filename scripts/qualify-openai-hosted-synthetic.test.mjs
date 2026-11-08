import test from 'node:test';
import assert from 'node:assert/strict';
import { syntheticRequest, parseStream, runSyntheticPilot } from './qualify-openai-hosted-synthetic.mjs';

const SESSION = 'sess_synthetic';
const TURN = 'turn_synthetic';
const ENV = 'env_synthetic';
const ARTIFACT = 'artifact_synthetic';
const OUTPUT = Buffer.from('infimount-synthetic-ok\n');

function json(data, status = 200) {
  return new Response(JSON.stringify(data), {
    status, headers: { 'content-type': 'application/json' },
  });
}

function events({ failed = false, truncated = false, includeDone = false } = {}) {
  const lines = [
    { type: 'agent.session.created', session: { id: SESSION, environment: { id: ENV } } },
    { type: 'agent.session.idle' },
    ...(truncated ? [] : [{ type: failed ? 'agent.session.failed' : 'agent.session.turn.completed',
      turn: failed ? undefined : { id: TURN, subagent_id: null } }]),
  ];
  return lines.map(value => `data: ${JSON.stringify(value)}\n\n`).join('')
    + (includeDone ? 'data: [DONE]\n\n' : '');
}

function streamedSse(text, chunkSize = 7) {
  const bytes = Buffer.from(text);
  let offset = 0;
  return new Response(new ReadableStream({
    pull(controller) {
      if (offset >= bytes.length) { controller.close(); return; }
      const end = Math.min(offset + chunkSize, bytes.length);
      controller.enqueue(bytes.subarray(offset, end));
      offset = end;
    },
  }), { headers: { 'content-type': 'text/event-stream; charset=utf-8' } });
}

function mockProvider({
  toolFailed = false,
  wrongPath = false,
  wrongBytes = false,
  crossTurn = false,
  truncated = false,
  lateFailure = false,
  secondItemsPage = false,
  repeatedCursor = false,
  wrongDelete = false,
} = {}) {
  const calls = [];
  async function fetchImpl(url, init) {
    const request = new URL(url);
    calls.push({ pathname: request.pathname, search: request.search, method: init.method });
    assert.equal(request.origin, 'https://api.openai.com');
    assert.equal(init.headers.Authorization, 'Bearer synthetic_key');
    assert.equal(init.headers['OpenAI-Beta'], 'agents=v1');
    assert.equal(init.redirect, 'error');
    if (request.pathname === '/v1/agents/sessions' && init.method === 'POST') {
      const body = JSON.parse(init.body);
      assert.equal(body.environment.type, 'openai_hosted');
      assert.equal(body.environment.network.access, 'disabled');
      assert.equal(body.environment.container_size, 'small');
      assert.equal(body.spend_control.limit, 100);
      assert.equal(body.environment.files, undefined);
      return streamedSse(events({ failed: lateFailure, truncated }), 5);
    }
    if (request.pathname === `/v1/agents/sessions/${SESSION}` && init.method === 'GET') {
      return json({ id: SESSION, object: 'agent.session', status: 'idle', environment: { id: ENV } });
    }
    if (request.pathname === `/v1/agents/sessions/${SESSION}/turns/${TURN}/items`) {
      if (secondItemsPage && request.searchParams.has('after')) {
        assert.equal(request.searchParams.get('after'), 'opaque:page:1');
        return json({ object: 'list', data: [
          { id: 'cmd_2', turn_id: TURN, type: 'command_execution', status: 'completed', exit_code: toolFailed ? 42 : 0 },
        ], first_id: 'cmd_2', last_id: repeatedCursor ? 'opaque:page:1' : 'opaque:page:2', has_more: false });
      }
      return json({ object: 'list', data: [
        { id: 'msg_1', turn_id: crossTurn ? 'turn_other' : TURN, type: 'message', status: 'completed' },
        ...secondItemsPage ? [] : [{ id: 'cmd_2', turn_id: TURN, type: 'command_execution', status: 'completed', exit_code: toolFailed ? 1 : 0 }],
      ], first_id: 'msg_1', last_id: secondItemsPage ? 'opaque:page:1' : 'opaque:page:2', has_more: secondItemsPage });
    }
    if (request.pathname === `/v1/agents/sessions/${SESSION}/artifacts`) {
      return json({ object: 'list', data: [{
        id: ARTIFACT,
        object: 'agent.session.artifact',
        session_id: SESSION,
        environment_id: ENV,
        turn_id: TURN,
        path: wrongPath ? '/workspace/outputs/private.txt' : '/workspace/outputs/infimount-pilot.txt',
        size_bytes: OUTPUT.length,
      }], first_id: ARTIFACT, last_id: 'opaque:artifact:1', has_more: false });
    }
    if (request.pathname === `/v1/agents/sessions/${SESSION}/artifacts/${ARTIFACT}/content`) {
      const bytes = Buffer.from(OUTPUT);
      if (wrongBytes) bytes[0] ^= 1;
      return new Response(bytes, { headers: { 'content-type': 'application/octet-stream' } });
    }
    if (request.pathname === `/v1/agents/sessions/${SESSION}` && init.method === 'DELETE') {
      return json({ id: SESSION, deleted: !wrongDelete, object: 'agent.session.deleted' });
    }
    throw Error(`unexpected mock route: ${request.pathname}`);
  }
  return { fetchImpl, calls };
}

const liveArgs = { apiKey: 'synthetic_key', authorized: true, confirmedResidency: true, confirmedSpend: true };

test('request is synthetic only, small sandbox, network denied and cost-bounded', () => {
  const request = syntheticRequest();
  assert.equal(request.spend_control.limit, 100);
  assert.equal(request.environment.container_size, 'small');
  assert.equal(request.environment.network.access, 'disabled');
  assert.equal(request.environment.files, undefined);
  assert.equal(request.agent.tools, undefined);
  assert.equal(request.stream, true);
});

test('partial/split stream captures exact root without requiring a [DONE] marker', () => {
  const value = parseStream(Buffer.from(events()));
  assert.deepEqual({ sessionId: value.sessionId, turnId: value.turnId }, { sessionId: SESSION, turnId: TURN });
  assert.deepEqual(parseStream(Buffer.from(events({ includeDone: true }))).sessionId, SESSION);
  assert.throws(() => parseStream(Buffer.from(events({ truncated: true }))));
  const subagent = events().replace('"subagent_id":null', '"subagent_id":"sub_1"');
  assert.throws(() => parseStream(Buffer.from(subagent)));
});

test('consent and key checks reject before any network request', async () => {
  const { fetchImpl, calls } = mockProvider();
  await assert.rejects(() => runSyntheticPilot({ ...liveArgs, fetchImpl, confirmedSpend: false }));
  await assert.rejects(() => runSyntheticPilot({ ...liveArgs, fetchImpl, confirmedResidency: false }));
  await assert.rejects(() => runSyntheticPilot({ ...liveArgs, fetchImpl, authorized: false }));
  await assert.rejects(() => runSyntheticPilot({ ...liveArgs, fetchImpl, apiKey: '' }));
  assert.equal(calls.length, 0);
});

test('success verifies root, tools, exact file bytes and deletes session', async () => {
  const { fetchImpl, calls } = mockProvider();
  const result = await runSyntheticPilot({ ...liveArgs, fetchImpl });
  assert.equal(result.qualified, true);
  assert.equal(result.sessionDeleted, true);
  assert.equal(result.commandExecutions, 1);
  assert.equal(result.files, 1);
  assert.equal(result.sha256.length, 64);
  assert.equal(calls.filter(call => call.method === 'POST').length, 1);
  assert.equal(calls.at(-1).method, 'DELETE');
});

test('a multi-page saved-turn listing uses opaque pagination and URL encoding', async () => {
  const { fetchImpl, calls } = mockProvider({ secondItemsPage: true });
  const result = await runSyntheticPilot({ ...liveArgs, fetchImpl });
  assert.equal(result.qualified, true);
  assert.equal(calls.some(call => call.search.includes('after=opaque%3Apage%3A1')), true);
});

test('nonzero tool exit, cross-turn saved items, bad artifact path and byte change fail closed', async () => {
  for (const caseInfo of [
    { toolFailed: true }, { crossTurn: true }, { wrongPath: true }, { wrongBytes: true },
  ]) {
    const { fetchImpl, calls } = mockProvider(caseInfo);
    await assert.rejects(() => runSyntheticPilot({ ...liveArgs, fetchImpl }));
    assert.equal(calls.at(-1).method, 'DELETE');
  }
});

test('a duplicate page cursor is rejected and session cleaned up', async () => {
  const { fetchImpl, calls } = mockProvider({ repeatedCursor: true, secondItemsPage: true });
  await assert.rejects(() => runSyntheticPilot({ ...liveArgs, fetchImpl }), /repeated opaque page cursor/);
  assert.equal(calls.at(-1).method, 'DELETE');
});

test('session creation ID is captured before an interrupted SSE finishes, then deleted', async () => {
  const { fetchImpl, calls } = mockProvider({ truncated: true });
  await assert.rejects(() => runSyntheticPilot({ ...liveArgs, fetchImpl }), /incomplete hosted event stream/);
  assert.equal(calls.at(-1).method, 'DELETE');
});

test('a provider failure after a root may not be treated as success', async () => {
  const body = events() + 'data: {"type":"agent.session.failed"}\n\n';
  assert.throws(() => parseStream(Buffer.from(body)), /provider session failed/);
  const { fetchImpl, calls } = mockProvider({ lateFailure: true });
  await assert.rejects(() => runSyntheticPilot({ ...liveArgs, fetchImpl }), /provider session failed/);
  assert.equal(calls.at(-1).method, 'DELETE');
});

test('a failed deletion is surfaced as cleanup required and never qualified', async () => {
  const { fetchImpl } = mockProvider({ wrongDelete: true });
  await assert.rejects(() => runSyntheticPilot({ ...liveArgs, fetchImpl }), /manual session cleanup required/);
});
