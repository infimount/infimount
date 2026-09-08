# Infimount Governance

Infimount is an early-stage open-source project. Its governance should describe how the project actually operates today, not imitate the structure of a large foundation project before that structure exists.

## Current maintainer

**Rajan Kadeval ([@roylkng](https://github.com/roylkng))** is the current project maintainer and is responsible for release decisions, repository administration, roadmap direction, and final merge decisions.

This role is operational, not permanent ownership of the community. Governance will evolve when sustained external participation makes broader decision-making useful.

## Contributors

Anyone who contributes code, documentation, issues, testing, review, design feedback, or reproducible bug reports is a contributor.

Contributors are encouraged to:

- open issues for material bugs or proposed changes;
- keep pull requests focused and reviewable;
- include tests or reproducible evidence where behavior changes;
- call out security, privacy, compatibility, or migration risks explicitly;
- challenge project assumptions with evidence.

See [CONTRIBUTING.md](CONTRIBUTING.md) for contribution mechanics and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) for community expectations.

## Decision making

Routine decisions are made in the relevant issue or pull request and should be explainable from the project goals, existing contracts, tests, and evidence.

For changes with significant compatibility, security, storage-integrity, or user-data implications, the maintainer may require additional review or retained validation evidence before merge.

The project prefers reversible decisions and explicit trade-offs over process for its own sake.

## Becoming a maintainer

As the contributor base grows, maintainership can be extended to contributors who demonstrate sustained technical judgment and responsibility for project health. Signals include:

1. repeated high-quality contributions over time;
2. constructive code and design review;
3. good judgment around compatibility, security, and user-data risks;
4. willingness to maintain features after initial implementation;
5. alignment with the project's local-first and explicit-control principles.

When additional maintainers exist, this document will name them and define any decision or voting model that is actually in use.

## Security-sensitive decisions

Security vulnerabilities should follow [SECURITY.md](SECURITY.md) rather than being disclosed first in a public issue.

Changes affecting MCP permissions, filesystem/path boundaries, credentials, updater trust, or destructive storage operations receive additional scrutiny because mistakes at those boundaries can affect user data or agent authority.

## Governance changes

This document evolves with the project. Material governance changes should be proposed publicly and include the reason the current model is no longer sufficient.
