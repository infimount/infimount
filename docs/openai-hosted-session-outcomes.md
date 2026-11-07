# Infimount v0.8.2 C2.2: offline hosted session outcome gate

Status: **pure event classification only**. No HTTP, SSE transport, session
creation, provider credentials, billing, artifact downloads or publication.

## Why the gate exists

OpenAI's Agents API is asynchronous. A reported `agent.session.idle` is
only readiness for another turn. A closed stream is not cancellation or
proof of success. A root turn that completed can still have failed tool calls,
missing files or unsafe artifacts. [Official event guide](https://developers.openai.com/api/docs/guides/agents-api/sessions/events).

The `openai_hosted_events::HostedSessionEventGate` is a passive, bounded
state machine fed with JSON events from a **trusted, authenticated,
session-scoped** provider transport. It is **not** intended for JSON
returned by a model or arbitrary downloaded files.

The outcome types are:

- `Pending` and `RequiresAction`: nonterminal, never authorized.
- `RootTurnCompletedUnverified`: the provider reports root turn completion,
  but Infimount has **not** inspected tool failures or artifacts. This is
  never evidence of successful processing or user-approved publication.
- `RootTurnFailed`, `RootTurnCancelled`, `SessionFailed`: terminal errors.
- `StreamInterrupted`: the stream closed before a root terminal event.
  A future trusted client must reconnect, read saved items and recover state
  before deciding what happened. No automatic retry of publication.

The provider's documented `event.turn.subagent_id === null` distinguishes
the root turn from subagents. Subagent completion never completes the root
turn. A missing or invalid discriminator is rejected, rather than guessed.

Session and environment failures always fail closed even if a completion
signal was previously observed. Contradictory root-terminal results also fail
closed rather than trusting an earlier completion. Unknown event types, text deltas and idle
signals do not advance the terminal state. Malformed events and exceeding
the maximum event size/count lead to a terminal failure, with no unbounded
event history retained.

## Next gates before real provider use

- Acquire event bytes only from an authenticated stream of the session
  created through the reviewed C2.1 request contract, with expected session
  and root turn binding.
- Handle `requires_action` using trusted control-plane authorization. Never
  let an agent call the final publication approval method.
- For root completion, retrieve the saved turn and paginate **all** items.
  Check reported tool failures, output tree and artifact list. Completion
  alone is insufficient.
- Independently download, inventory and rehash exact artifact bytes under
  C1.2/C1.3 confinement rules. Refuse unexpected files and unsafe types.
- Ensure bounded session retention/deletion and cancellation semantics.
  Provider stream disconnect is not a cancellation mechanism.
- Require separate human review before publishing into an original storage.

Source: [Agents API events](https://developers.openai.com/api/docs/guides/agents-api/sessions/events),
[sessions](https://developers.openai.com/api/docs/guides/agents-api/sessions)
and [OpenAI-hosted environments](https://developers.openai.com/api/docs/guides/agents-api/environments/openai-hosted).

No v0.8.1 stable modification or headless storage publication permission.
