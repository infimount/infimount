#!/usr/bin/env node
// Infimount v0.8.2 C2 live interoperability qualification, opt-in and synthetic-only.
// This is NOT a production adapter or a publication authority.
import { createHash } from 'node:crypto';
import { pathToFileURL } from 'node:url';

const BASE = 'https://api.openai.com/v1';
const MODEL = 'gpt-6-astra';
const OUTPUT_PATH = '/workspace/outputs/infimount-pilot.txt';
const OUTPUT = Buffer.from('infimount-synthetic-ok\n', 'utf8');
const MAX_JSON_BYTES = 2 * 1024 * 1024;
const MAX_STREAM_BYTES = 8 * 1024 * 1024;
const MAX_PAGES = 50;
const MAX_EVENTS = 10_000;
const MAX_SPEND_CENTS = 100;
const OPAQUE_ID = /^[A-Za-z0-9_-]{1,256}$/;

export function syntheticRequest() {
  return {
    agent: {
      model: MODEL,
      instructions: 'Use only the task instructions. Do not request network access, credentials, MCP tools, or external file mounts. Write only the named output file.',
    },
    environment: {
      type: 'openai_hosted',
      container_size: 'small',
      network: { access: 'disabled' },
    },
    input: 'Create one file at /workspace/outputs/infimount-pilot.txt with the exact UTF-8 bytes infimount-synthetic-ok followed by a single LF newline. Verify the bytes using a local command and report the command result. Do not create any additional output files.',
    spend_control: { limit: MAX_SPEND_CENTS },
    stream: true,
  };
}

function requireId(value, name) {
  if (typeof value !== 'string' || !OPAQUE_ID.test(value)) {
    throw Error(`invalid ${name}`);
  }
  return value;
}

function assertOk(response) {
  if (!response?.ok) throw Error(`provider HTTP ${response?.status ?? 'unknown'}`);
}

function checkLength(response, cap) {
  const size = response.headers?.get('content-length');
  if (size !== null && size !== undefined && /^\d+$/.test(size) && Number(size) > cap) {
    throw Error('provider response exceeded size limit');
  }
}

async function boundedBytes(response, cap) {
  assertOk(response);
  checkLength(response, cap);
  const reader = response.body?.getReader();
  if (!reader) throw Error('provider response missing body');
  const chunks = [];
  let total = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      total += value.byteLength;
      if (total > cap) throw Error('provider response exceeded byte limit');
      chunks.push(Buffer.from(value));
    }
    return Buffer.concat(chunks);
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}

function decodeJson(bytes) {
  try { return JSON.parse(bytes.toString('utf8')); }
  catch { throw Error('malformed provider JSON'); }
}

// Streaming parser captures the session ID as soon as its event arrives.
// If a later event fails, the caller can still request session deletion.
function streamGate(onSession) {
  let sessionId = null;
  let environmentId = null;
  let turnId = null;
  let completed = false;
  let done = false;
  let events = 0;
  let textBuffer = '';
  let dataLines = [];

  function eventFrame() {
    if (dataLines.length === 0) return;
    const data = dataLines.join('\n');
    dataLines = [];
    if (done) throw Error('provider sent event after SSE completion');
    if (data === '[DONE]') { done = true; return; }
    if (++events > MAX_EVENTS || Buffer.byteLength(data) > MAX_JSON_BYTES) {
      throw Error('SSE event budget exceeded');
    }
    let event;
    try { event = JSON.parse(data); }
    catch { throw Error('malformed SSE event'); }
    if (!event || typeof event.type !== 'string') throw Error('missing SSE event type');
    if (event.type === 'agent.session.created') {
      if (sessionId) throw Error('duplicate session creation event');
      sessionId = requireId(event.session?.id, 'session ID');
      onSession(sessionId);
      if (event.session?.environment?.id) {
        environmentId = requireId(event.session.environment.id, 'environment ID');
      }
    }
    if (['error', 'agent.session.failed', 'agent.session.environment.failed'].includes(event.type)) {
      throw Error('provider session failed');
    }
    if (['agent.session.requires_action', 'agent.session.action_required'].includes(event.type)) {
      throw Error('provider requires unexpected action');
    }
    if (['agent.session.turn.completed', 'agent.session.turn.failed', 'agent.session.turn.cancelled'].includes(event.type)) {
      if (!event.turn || !Object.hasOwn(event.turn, 'subagent_id')) {
        throw Error('root-turn discriminator missing');
      }
      if (event.turn.subagent_id !== null) return;
      if (!sessionId || completed || event.type !== 'agent.session.turn.completed') {
        throw Error('root turn failed, cancelled or contradictory');
      }
      turnId = requireId(event.turn.id, 'root turn ID');
      completed = true;
    }
  }

  function line(value) {
    if (value === '') { eventFrame(); return; }
    if (value.startsWith('data:')) {
      dataLines.push(value.slice(5).trimStart());
      if (Buffer.byteLength(dataLines.join('\n')) > MAX_JSON_BYTES) {
        throw Error('SSE frame exceeded byte limit');
      }
    }
    // Other SSE fields (id, event, retry, comments) cannot authorize success.
  }

  return {
    push(fragment) {
      textBuffer += fragment;
      if (Buffer.byteLength(textBuffer) > MAX_JSON_BYTES * 2) {
        throw Error('SSE input frame buffer exceeded byte limit');
      }
      let offset;
      while ((offset = textBuffer.indexOf('\n')) >= 0) {
        const next = textBuffer.slice(0, offset).replace(/\r$/, '');
        textBuffer = textBuffer.slice(offset + 1);
        line(next);
      }
    },
    finish() {
      if (textBuffer.length > 0) {
        line(textBuffer.replace(/\r$/, ''));
        textBuffer = '';
      }
      eventFrame();
      // The API examples do not promise a [DONE] SSE marker. A root turn
      // completed event AND clean stream closure are required instead.
      if (!sessionId || !turnId || !completed) throw Error('incomplete hosted event stream');
      return { sessionId, turnId, environmentId, eventCount: events };
    },
  };
}

export function parseStream(bytes, onSession = () => {}) {
  if (bytes.byteLength > MAX_STREAM_BYTES) throw Error('SSE stream exceeded byte limit');
  const gate = streamGate(onSession);
  gate.push(Buffer.from(bytes).toString('utf8'));
  return gate.finish();
}

async function consumeStream(response, onSession) {
  assertOk(response);
  checkLength(response, MAX_STREAM_BYTES);
  const type = response.headers?.get('content-type');
  if (type && !type.toLowerCase().includes('text/event-stream')) {
    throw Error('provider did not return an SSE stream');
  }
  const reader = response.body?.getReader();
  if (!reader) throw Error('provider event stream missing body');
  const gate = streamGate(onSession);
  const decoder = new TextDecoder('utf-8', { fatal: true });
  let seenBytes = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      seenBytes += value.byteLength;
      if (seenBytes > MAX_STREAM_BYTES) throw Error('SSE stream exceeded byte limit');
      gate.push(decoder.decode(value, { stream: true }));
    }
    gate.push(decoder.decode());
    return gate.finish();
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}

async function request(fetchImpl, key, suffix, options = {}) {
  if (!suffix.startsWith('/agents/') || suffix.includes('..')) {
    throw Error('invalid provider route');
  }
  const headers = {
    Authorization: `Bearer ${key}`,
    'OpenAI-Beta': 'agents=v1',
  };
  if (options.body) headers['Content-Type'] = 'application/json';
  return fetchImpl(`${BASE}${suffix}`, {
    method: options.method || 'GET',
    headers,
    body: options.body ? JSON.stringify(options.body) : undefined,
    signal: AbortSignal.timeout(180_000),
    redirect: 'error',
  });
}

async function listAll(fetchImpl, key, basePath, maxItems) {
  const items = [];
  const itemIds = new Set();
  const cursors = new Set();
  let after = null;
  for (let page = 0; page < MAX_PAGES; page++) {
    const query = new URLSearchParams({ order: 'asc', limit: '100' });
    if (after) query.set('after', after);
    const res = await request(fetchImpl, key, `${basePath}?${query.toString()}`);
    const value = decodeJson(await boundedBytes(res, MAX_JSON_BYTES));
    if (value.object !== 'list' || !Array.isArray(value.data)
        || value.data.length === 0 || value.data.length > 100
        || typeof value.has_more !== 'boolean') {
      throw Error('malformed provider listing');
    }
    if (value.first_id !== value.data[0]?.id) throw Error('invalid first item identity');
    const cursor = value.last_id;
    if (typeof cursor !== 'string' || !cursor.length || cursor.length > 1024
        || !/^[\x21-\x7e]+$/.test(cursor) || cursors.has(cursor)) {
      throw Error('invalid/repeated opaque page cursor');
    }
    cursors.add(cursor);
    for (const item of value.data) {
      if (!OPAQUE_ID.test(item.id) || itemIds.has(item.id)) {
        throw Error('duplicate or invalid saved item identity');
      }
      itemIds.add(item.id);
      items.push(item);
      if (items.length > maxItems) throw Error('too many provider records');
    }
    if (!value.has_more) return items;
    after = cursor; // Opaque. URLSearchParams encodes it safely.
  }
  throw Error('too many provider list pages');
}

function checkSavedItems(items, turnId) {
  let commands = 0;
  for (const item of items) {
    if (item.turn_id !== turnId) throw Error('cross-turn saved item');
    if (item.type === 'command_execution') {
      if (item.status !== 'completed' || item.exit_code !== 0) {
        throw Error('provider command did not complete successfully');
      }
      commands++;
    } else if (item.type === 'message' || item.type === 'reasoning') {
      if (['failed', 'incomplete', 'in_progress'].includes(item.status)) {
        throw Error('unfinished or failed saved item');
      }
    } else {
      throw Error('unsupported provider tool type');
    }
  }
  if (commands < 1) throw Error('provider did not report a successful verification command');
  return commands;
}

async function deleteSession(fetchImpl, key, sessionId) {
  for (let attempt = 0; attempt < 4; attempt++) {
    const response = await request(fetchImpl, key, `/agents/sessions/${sessionId}`, { method: 'DELETE' });
    if (response.status === 409 && attempt < 3) {
      await new Promise(resolve => setTimeout(resolve, Math.min(1000 * (attempt + 1), 3000)));
      continue;
    }
    const data = decodeJson(await boundedBytes(response, MAX_JSON_BYTES));
    if (data.deleted === true && data.id === sessionId
        && data.object === 'agent.session.deleted') return true;
    throw Error('session deletion was not confirmed');
  }
  throw Error('session deletion retries exhausted');
}

export async function runSyntheticPilot({
  fetchImpl = fetch,
  apiKey,
  authorized = false,
  confirmedResidency = false,
  confirmedSpend = false,
} = {}) {
  if (!authorized || !confirmedResidency || !confirmedSpend) {
    throw Error('live execution, U.S. residency/non-ZDR and $1 spending must be explicitly acknowledged');
  }
  if (typeof apiKey !== 'string' || !apiKey.trim()) {
    throw Error('OPENAI_API_KEY is required for the opt-in pilot');
  }
  let sessionId = null;
  let result = null;
  let failure = null;
  try {
    // Exactly one session creation attempt. Never automatically retry a paid
    // POST because a network timeout might occur after successful creation.
    const creation = await request(fetchImpl, apiKey, '/agents/sessions', {
      method: 'POST', body: syntheticRequest(),
    });
    const observed = await consumeStream(creation, id => { sessionId = id; });
    sessionId = observed.sessionId;
    const sessionResponse = await request(fetchImpl, apiKey, `/agents/sessions/${sessionId}`);
    const session = decodeJson(await boundedBytes(sessionResponse, MAX_JSON_BYTES));
    if (session.id !== sessionId || session.status !== 'idle'
        || session.environment?.id !== observed.environmentId && observed.environmentId !== null) {
      throw Error('hosted session identity or status mismatch');
    }
    const environmentId = requireId(session.environment?.id, 'environment ID');
    const saved = await listAll(fetchImpl, apiKey,
      `/agents/sessions/${sessionId}/turns/${observed.turnId}/items`, 5000);
    const commandExecutions = checkSavedItems(saved, observed.turnId);
    const artifacts = await listAll(fetchImpl, apiKey,
      `/agents/sessions/${sessionId}/artifacts`, 100);
    if (artifacts.length !== 1) throw Error('unexpected output artifact count');
    const artifact = artifacts[0];
    if (artifact.object !== 'agent.session.artifact' || artifact.session_id !== sessionId
        || artifact.environment_id !== environmentId || artifact.turn_id !== observed.turnId
        || artifact.path !== OUTPUT_PATH || artifact.size_bytes !== OUTPUT.length) {
      throw Error('output artifact does not match the reviewed synthetic contract');
    }
    const id = requireId(artifact.id, 'artifact ID');
    const response = await request(fetchImpl, apiKey,
      `/agents/sessions/${sessionId}/artifacts/${id}/content`);
    const data = await boundedBytes(response, OUTPUT.length);
    if (!data.equals(OUTPUT)) throw Error('downloaded artifact bytes differ from expected output');
    result = {
      qualified: true,
      model: MODEL,
      files: 1,
      bytes: data.length,
      sha256: createHash('sha256').update(data).digest('hex'),
      commandExecutions,
      sessionDeleted: true,
    };
  } catch (error) {
    failure = error;
  } finally {
    if (sessionId) {
      try { await deleteSession(fetchImpl, apiKey, sessionId); }
      catch (cleanupError) {
        throw new Error(`manual session cleanup required for ${sessionId}: ${cleanupError.message}`,
          { cause: failure ?? cleanupError });
      }
    }
  }
  if (failure) throw failure;
  if (!sessionId) throw Error('session creation status is unknown, inspect the Agents dashboard');
  return result;
}

function flags(argv) {
  const allowed = new Set([
    '--execute', '--accept-us-data-residency-and-non-zdr',
    '--approve-one-usd-spend', '--help',
  ]);
  if (argv.some(arg => !allowed.has(arg)) || new Set(argv).size !== argv.length) {
    throw Error('unrecognized or duplicate qualification flag');
  }
  return new Set(argv);
}

async function main() {
  const args = flags(process.argv.slice(2));
  if (args.has('--help')) {
    console.log('Offline: node scripts/qualify-openai-hosted-synthetic.mjs');
    console.log('Live synthetic only: node scripts/qualify-openai-hosted-synthetic.mjs --execute --accept-us-data-residency-and-non-zdr --approve-one-usd-spend');
    return;
  }
  if (!args.has('--execute')) {
    console.log(JSON.stringify({
      mode: 'offline-preview',
      networkCalls: 0,
      provider: 'OpenAI managed Agents API beta',
      syntheticOnly: true,
      model: MODEL,
      usResidencyOnly: true,
      zeroDataRetention: false,
      networkAccess: 'disabled',
      spendLimitUsdCents: MAX_SPEND_CENTS,
      outputPath: OUTPUT_PATH,
    }, null, 2));
    return;
  }
  const result = await runSyntheticPilot({
    apiKey: process.env.OPENAI_API_KEY,
    authorized: true,
    confirmedResidency: args.has('--accept-us-data-residency-and-non-zdr'),
    confirmedSpend: args.has('--approve-one-usd-spend'),
  });
  console.log(JSON.stringify(result, null, 2));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(error => {
    console.error(`Hosted qualification did not complete: ${error.message}`);
    process.exitCode = 1;
  });
}
