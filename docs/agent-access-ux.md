# Agent Access product semantics

This document defines the user-facing Agent Access model for Infimount.

## Normal flow

Storage browsing is independent from agent access.

`Storage -> Workspace -> Agent Access -> Connect Client -> Verify`

A user may choose storage-only usage and complete onboarding without configuring an agent.

## Readiness states

Infimount presents readiness as separate dimensions rather than one generic activation flag:

- Storage: no storage / ready
- Workspace: none / scoped
- Agent Access: disabled / ready / needs attention
- Client: not configured / configured / verified
- HTTP runtime: stopped / running
- stdio runtime: on demand

## stdio

stdio does not have a persistent background server. A configured MCP client launches the bundled Infimount sidecar when it connects.

User-facing status must therefore say `On demand` or `Ready for client launch`, never `Stopped`.

The persisted MCP `enabled` flag is the general Agent Access gate. When it is false, the general sidecar `serve` command must fail closed before exposing tools. Agent Task's separately scoped `serve-agent-task` command remains independent.

## HTTP

HTTP is a persistent runtime. It has explicit Start and Stop controls, an endpoint, bind-address safety rules, and authentication requirements for non-loopback use.

HTTP process state must never be presented as the generic state of all MCP access.

## Advanced MCP settings

Advanced settings retain explicit tool selection, policies, confirmations, sessions, audit, transport details, authentication, and raw client snippets. Normal users should not need this surface just to connect a client to one workspace.

## Notices

Transient toasts may auto-dismiss. Any warning, alert, banner, bar, or toast that does not auto-dismiss must provide an obvious accessible close/dismiss control. Persistent product readiness is represented through status surfaces, not undismissable floating warnings.
