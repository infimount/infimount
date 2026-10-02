# Infimount Governance

Infimount is an early-stage open-source project. This document describes how decisions are actually made today and should evolve with the contributor base rather than pre-declare organizational structure that does not yet exist.

## Current maintainer

**Rajan Kadeval ([@roylkng](https://github.com/roylkng))** is the current project maintainer and is responsible for repository administration, release decisions, roadmap direction, and final merge decisions.

This is an operational responsibility, not permanent ownership of the community. Governance should broaden when sustained external participation makes shared maintainership useful.

## Contributors

Anyone who contributes code, documentation, issues, testing, review, design feedback, or reproducible bug reports is a contributor.

Contributors are encouraged to:

- open issues for material bugs or proposed changes;
- keep pull requests focused and reviewable;
- include tests or reproducible evidence where behavior changes;
- call out security, privacy, compatibility, migration, and user-data risks explicitly;
- challenge assumptions with evidence.

See [CONTRIBUTING.md](CONTRIBUTING.md) for contribution mechanics and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) for community expectations.

## Decision making

Routine decisions are made in the relevant issue or pull request and should be explainable from project goals, documented contracts, tests, and evidence.

For changes with significant compatibility, security, storage-integrity, agent-authority, or user-data implications, the maintainer may require additional review or retained validation evidence before merge.

The project prefers reversible decisions and explicit trade-offs over process for its own sake.

## Becoming a maintainer

As the contributor base grows, maintainership can be extended to contributors who demonstrate sustained technical judgment and responsibility for project health. Signals include:

1. repeated high-quality contributions over time;
2. constructive code and design review;
3. good judgment around compatibility, security, agent authority, and user-data risks;
4. willingness to maintain features after initial implementation;
5. alignment with the project's local-first and explicit-control principles.

When additional maintainers exist, [MAINTAINERS.md](MAINTAINERS.md) will name them and this document will define any shared decision or voting model that is actually in use.

## Security-sensitive decisions

Security vulnerabilities should follow [SECURITY.md](SECURITY.md) rather than being disclosed first in a public issue.

Changes affecting MCP permissions, filesystem/path boundaries, credentials, updater trust, recovery, or destructive storage operations receive additional scrutiny because mistakes at those boundaries can affect user data or agent authority.

## Governance changes

This document evolves with the project. Material governance changes should be proposed publicly and explain why the current model is no longer sufficient.
